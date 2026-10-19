use crate::{
    Error, Result,
    conversation::{ConversationJob, ConversationPage, Message},
    github::Response,
    runtime::Runtime,
    transport::ReadOp,
};
use tokio_util::sync::CancellationToken;

impl Runtime {
    /// Immediate user submission, deliberately excluded from the retry queue.
    pub async fn post_conversation(
        &self,
        job: ConversationJob,
        body: String,
        thread: Option<String>,
    ) -> Result<Message> {
        job.validate()?;
        if body.trim().is_empty() || body.chars().count() > 65536 || body.len() > 128 * 1024 {
            return Err(Error::Safe(
                "Write a comment of at most 65,536 characters (128 KiB).".into(),
            ));
        }
        let (transport, generation) = {
            let mut i = self.lock()?;
            if i.locked
                || !i.reader_open
                || i.selected.as_ref() != Some(&(job.repo.clone(), job.number))
                || i.store.conversation_denied(&job.repo, job.number)?
            {
                return Err(Error::Denied);
            }
            Self::require_remote_storage(&i)?;
            if let Some(thread) = &thread {
                let page = i
                    .store
                    .get::<ConversationPage>(&job.key())?
                    .data
                    .ok_or(Error::Denied)?;
                if !(page.threads.iter().any(|t| &t.id == thread)
                    || (job.source == "replies"
                        && job.thread.as_ref() == Some(thread)
                        && !page.messages.is_empty()))
                {
                    return Err(Error::Denied);
                }
            }
            (
                i.transport.clone().ok_or(Error::Denied)?,
                i.store.generation()?,
            )
        };
        let cancel = CancellationToken::new();
        self.identity(&transport, &cancel, generation).await?;
        let overview = self
            .response(
                &transport,
                ReadOp::Overview {
                    repo: job.repo.clone(),
                    number: job.number,
                },
                &cancel,
            )
            .await?;
        overview.require_ok()?;
        if overview.body["head"]["sha"].as_str() != Some(&job.head) {
            return Err(Error::Safe(
                "Refresh the PR before commenting on this revision.".into(),
            ));
        }
        let subject = thread
            .as_deref()
            .or_else(|| overview.body["node_id"].as_str())
            .ok_or(Error::Denied)?;
        {
            let i = self.lock()?;
            if i.locked
                || i.store.generation()? != generation
                || !i.reader_open
                || i.selected.as_ref() != Some(&(job.repo.clone(), job.number))
            {
                return Err(Error::Denied);
            }
            if !i.store.budget_eligible("graphql", crate::now())? {
                return Err(Error::Safe(
                    "GitHub is rate limiting requests. Wait before posting.".into(),
                ));
            }
        }
        let output = transport
            .post_comment(subject, thread.is_some(), &body, &cancel)
            .await
            .map_err(|_| {
                Error::Safe(
                    "Couldn’t confirm posting. Check GitHub before retrying to avoid a duplicate."
                        .into(),
                )
            })?;
        let bytes = output.stdout.len();
        let response = Response::parse(output)?;
        self.lock()?
            .store
            .record_response("graphql", &response, bytes, crate::now())?;
        response.require_ok()?;
        if response.body["errors"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
        {
            return Err(Error::Safe("GitHub couldn’t post this comment. Check access and the discussion on GitHub before retrying.".into()));
        }
        let node = if thread.is_some() {
            &response.body["data"]["addPullRequestReviewThreadReply"]["comment"]
        } else {
            &response.body["data"]["addComment"]["commentEdge"]["node"]
        };
        let required = |key: &str| {
            node[key].as_str().map(String::from).ok_or_else(|| {
                Error::Safe("Couldn’t confirm posting. Check GitHub before retrying.".into())
            })
        };
        Ok(Message {
            id: required("id")?,
            kind: if thread.is_some() { "reply" } else { "comment" }.into(),
            author: node["author"]["login"].as_str().map(String::from),
            time: required("createdAt")?,
            url: Some(required("url")?),
            body: Some(body.clone()),
            state: "complete".into(),
            bytes: Some(body.len()),
            digest: None,
        })
    }
}
