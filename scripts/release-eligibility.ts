import { appendFileSync, readFileSync } from "node:fs";

type Release = { tag_name: string; draft: boolean };

function versionFromTag(tag: string): string {
  const version = tag.replace(/^v/, "");
  const number = "(?:0|[1-9][0-9]*)";
  const identifier = "(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)";
  const pattern = new RegExp(`^${number}\\.${number}\\.${number}(?:-${identifier}(?:\\.${identifier})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`);
  if (!pattern.test(version)) throw new Error(`Invalid release version: ${tag}`);
  return version;
}

export function releaseEligibility(current: string, releases: Release[]) {
  const version = versionFromTag(current);
  let highest: string | undefined;
  for (const release of releases) {
    // Drafts remain retryable; published prereleases count toward the maximum.
    if (release.draft) continue;
    const candidate = versionFromTag(release.tag_name);
    if (!highest || Bun.semver.order(candidate, highest) > 0) highest = candidate;
  }
  return { shouldRelease: !highest || Bun.semver.order(version, highest) > 0, highest };
}

if (import.meta.main) {
  const current = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")).version;
  // gh api --paginate --slurp returns one nested array per response page.
  const pages: Release[][] = JSON.parse(readFileSync(process.argv[2], "utf8"));
  const { shouldRelease, highest } = releaseEligibility(current, pages.flat());
  const message = `${shouldRelease ? "Release eligible" : "Skipping release"}: current ${current}, highest published ${highest ?? "none"}.`;
  console.log(message);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `should_release=${shouldRelease}\n`);
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${message}\n`);
}
