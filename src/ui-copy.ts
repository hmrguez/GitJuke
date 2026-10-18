// Keep runtime diagnostics out of everyday UI while preserving an actionable next step.
export function friendlyError(value:unknown,fallback='Something went wrong. Please try again.'):string{
 const text=String(value);
 if(/storage|disk|sqlite|database|readonly|read-only|no space/i.test(text))return 'Local storage needs attention. Open Preferences to review it.';
 if(/identity|auth|credential|login|forbidden|access denied|permission denied/i.test(text))return 'Check your GitHub connection in Preferences.';
 if(/rate.?limit|deadline|backoff|quota|retry.after/i.test(text))return 'GitHub is asking us to wait. Updates will resume automatically.';
 if(/network|offline|connection|timeout|timed out|resolve host/i.test(text))return 'Couldn’t reach GitHub. Check your connection and try again.';
 if(/version|changed|conflict|stale/i.test(text))return 'This has changed since you opened it. Refresh and try again.';
 return fallback;
}
export function activityLabel(kind:string):string{
 return ({comment:'Comment',review:'Review',mention:'Mention',check:'Check update',review_request:'Review request',request:'Review request'} as Record<string,string>)[kind]??'PR update';
}
