//! Explicit read-only compatibility probe. Prints only identity and counts, never PR payloads.
use gitjuke_core::{
    Result,
    github::{self, Response},
    transport::{ReadOp, Transport},
};
use tokio_util::sync::CancellationToken;
#[tokio::main]
async fn main() -> Result<()> {
    let t = Transport::discover(None, None)?;
    let c = CancellationToken::new();
    let v = t.read(ReadOp::Version, &c).await?;
    println!(
        "{}",
        String::from_utf8_lossy(&v.stdout)
            .lines()
            .next()
            .unwrap_or_default()
    );
    let login = github::parse_auth(t.read(ReadOp::AuthStatus, &c).await?)?;
    let id = github::parse_identity(&Response::parse(t.read(ReadOp::Identity, &c).await?)?)?;
    assert_eq!(login, id.login);
    println!("Stored identity: github.com / {} / {}", id.login, id.id);
    let r = Response::parse(
        t.read(
            ReadOp::Authored {
                login: id.login.clone(),
                page: 1,
            },
            &c,
        )
        .await?,
    )?;
    let p = github::parse_search(&r)?;
    println!(
        "Authored search: HTTP {}, {} loaded, {} known, incomplete={}",
        r.status,
        p.prs.len(),
        p.total,
        p.incomplete
    );
    println!(
        "API quota: remaining={}, reset={}",
        r.headers
            .get("x-ratelimit-remaining")
            .map(String::as_str)
            .unwrap_or("unknown"),
        r.headers
            .get("x-ratelimit-reset")
            .map(String::as_str)
            .unwrap_or("unknown")
    );
    let args: Vec<String> = std::env::args().collect();
    let fixture = if args.len() == 3 {
        Some((args[1].clone(), args[2].parse::<u64>().expect("PR number")))
    } else {
        p.prs.first().map(|pr| (pr.repo.clone(), pr.number))
    };
    if let Some((repo, number)) = fixture {
        assert!(github::valid_repo(&repo));
        let r = Response::parse(
            t.read(
                ReadOp::Overview {
                    repo: repo.clone(),
                    number,
                },
                &c,
            )
            .await?,
        )?;
        let pr = github::parse_overview(&r, &repo, number)?;
        println!(
            "Overview schema: HTTP {}, head present={}, nullable author={}, description limited={}",
            r.status,
            !pr.head.is_empty(),
            pr.author.is_none(),
            pr.body_limited
        );
        let r = Response::parse(
            t.read(
                ReadOp::Stacks {
                    repo: repo.clone(),
                    number,
                },
                &c,
            )
            .await?,
        )?;
        match github::stack_number(&r) {
            Ok(Some(stack)) => {
                let r = Response::parse(
                    t.read(
                        ReadOp::StackDetail {
                            repo,
                            number: stack,
                        },
                        &c,
                    )
                    .await?,
                )?;
                let state = github::parse_stack(&r, &pr.repo, number)?;
                println!(
                    "Native stack detailed schema validated: {}",
                    matches!(state, gitjuke_core::model::StackState::Published { .. })
                );
            }
            Ok(None) => println!("Native stack: HTTP {}, confirmed nonmember", r.status),
            Err(e) => println!(
                "Native stack compatibility BLOCKED: HTTP {} — {}",
                r.status, e
            ),
        }
    } else {
        println!(
            "PR/stack fixture: BLOCKED — no open authored PR; pass an authorized repository and PR number to this probe."
        );
    }
    let after = github::parse_identity(&Response::parse(t.read(ReadOp::Identity, &c).await?)?)?;
    assert_eq!(id.id, after.id);
    println!("Stable identity verified after reads; no credentials or GitHub content modified.");
    Ok(())
}
