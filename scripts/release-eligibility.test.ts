import { expect, test } from "bun:test";
import { releaseEligibility } from "./release-eligibility";

const published = (...tags: string[]) => tags.map(tag_name => ({ tag_name, draft: false }));

test("allows the first release and strictly newer versions", () => {
  expect(releaseEligibility("0.1.0", []).shouldRelease).toBe(true);
  expect(releaseEligibility("0.10.0", published("v0.9.0")).shouldRelease).toBe(true);
});

test("skips equal and older versions against the highest, regardless of list order", () => {
  const releases = published("v0.9.0", "v0.2.0", "v0.8.0");
  expect(releaseEligibility("0.9.0", releases).shouldRelease).toBe(false);
  expect(releaseEligibility("0.3.0", releases).shouldRelease).toBe(false);
  expect(releaseEligibility("0.9.0+build.2", releases).shouldRelease).toBe(false);
});

test("compares prereleases using semantic version precedence", () => {
  expect(releaseEligibility("1.0.0-beta.10", published("v1.0.0-beta.9")).shouldRelease).toBe(true);
  expect(releaseEligibility("1.0.0-beta.1", published("v1.0.0")).shouldRelease).toBe(false);
  expect(releaseEligibility("1.0.0", published("v1.0.0-rc.1")).shouldRelease).toBe(true);
});

test("allows retrying drafts and fails closed for unrecognized published versions", () => {
  expect(releaseEligibility("0.1.0", [{ tag_name: "v0.1.0", draft: true }]).shouldRelease).toBe(true);
  expect(() => releaseEligibility("0.1.0", published("release-2"))).toThrow();
});
