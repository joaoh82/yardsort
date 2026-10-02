import { describe, expect, it } from "vitest";
import { project, pullRequest, pullRequestsOf } from "@/test/fixtures";
import {
  authorsOf,
  DEFAULT_FILTERS,
  filtering,
  ME,
  readFilters,
  reviewDecision,
  visible,
  type Filters,
} from "./filters";
import { rowsOf } from "./rows";

const alpha = project("alpha");
const beta = project("beta");

/** `ada` is who gh is logged in as on alpha; on beta nobody has said yet. */
const rows = rowsOf([alpha, beta], {
  [alpha.id]: pullRequestsOf([
    pullRequest(1, { title: "Fix the login redirect", author: "ada", branch: "ys/login" }),
    pullRequest(2, {
      title: "Add a retry",
      author: "grace",
      details: {
        review: "REVIEW_REQUIRED",
        reviewRequests: [
          { name: "ada", team: false },
          { name: "ada", team: true },
        ],
      },
    }),
    pullRequest(3, {
      title: "Tidy settings",
      author: "Linus",
      details: { review: "APPROVED", reviews: [{ login: "Ada", state: "approved" }] },
    }),
    pullRequest(4, {
      title: "Bump the parser",
      author: "grace",
      state: "merged",
      details: { reviews: [{ login: "ken", state: "changesRequested" }] },
    }),
    pullRequest(5, { title: "Drop the flag", author: "grace", state: "closed" }),
    pullRequest(12, { title: "Draft of 5 things", author: "ken", draft: true }),
  ]),
  [beta.id]: pullRequestsOf(
    [
      pullRequest(1, {
        title: "Speed up paint",
        author: "ada",
        details: { reviewRequests: [{ name: "reviewers", team: true }] },
      }),
    ],
    { viewer: null },
  ),
});

const all: Filters = { ...DEFAULT_FILTERS, state: "all" };
const numbers = (filters: Partial<Filters>, search = "") =>
  visible(rows, { ...all, ...filters }, search)
    .map((row) => `${row.project.name}#${row.pr.number}`)
    .sort();

describe("visible", () => {
  it("starts on the open ones, and a draft is open", () => {
    expect(visible(rows, DEFAULT_FILTERS, "").every((row) => row.pr.state === "open")).toBe(true);
    expect(numbers({ state: "open" })).toEqual([
      "alpha#1",
      "alpha#12",
      "alpha#2",
      "alpha#3",
      "beta#1",
    ]);
    expect(numbers({ state: "merged" })).toEqual(["alpha#4"]);
    expect(numbers({ state: "closed" })).toEqual(["alpha#5"]);
    expect(numbers({})).toHaveLength(7);
  });

  it("narrows to the projects chosen, and to all of them when none is", () => {
    expect(numbers({ projects: [beta.id] })).toEqual(["beta#1"]);
    expect(numbers({ projects: [alpha.id, beta.id] })).toHaveLength(7);
    expect(numbers({ projects: [] })).toHaveLength(7);
  });

  it("narrows to an author, whatever the case, and to you where gh has said who you are", () => {
    expect(numbers({ author: "linus" })).toEqual(["alpha#3"]);
    expect(numbers({ author: "grace" })).toEqual(["alpha#2", "alpha#4", "alpha#5"]);
    // On beta nobody knows who "me" is, so ada's pull request there is not claimed as mine.
    expect(numbers({ author: ME })).toEqual(["alpha#1"]);
  });

  it("filters by where a pull request stands with its reviewers", () => {
    expect(numbers({ reviews: "required" })).toEqual(["alpha#2"]);
    expect(numbers({ reviews: "approved" })).toEqual(["alpha#3"]);
    expect(numbers({ reviews: "changes" })).toEqual(["alpha#4"]);
    expect(numbers({ reviews: "none" })).toEqual([
      "alpha#1",
      "alpha#12",
      "alpha#2",
      "alpha#5",
      "beta#1",
    ]);
  });

  it("knows which ones are about you", () => {
    expect(numbers({ reviews: "byYou" })).toEqual(["alpha#3"]);
    // Asked of ada herself — not of a team that happens to be called ada.
    expect(numbers({ reviews: "awaitingYou" })).toEqual(["alpha#2"]);
    // Not your own, not one you reviewed, and nothing at all where nobody knows who you are.
    expect(numbers({ reviews: "notByYou" })).toEqual(["alpha#12", "alpha#2", "alpha#4", "alpha#5"]);
  });

  it("searches titles, branches and numbers", () => {
    expect(numbers({}, "LOGIN")).toEqual(["alpha#1"]);
    expect(numbers({}, "ys/login")).toEqual(["alpha#1"]);
    expect(numbers({}, "#1")).toEqual(["alpha#1", "alpha#12", "beta#1"]);
    expect(numbers({}, "12")).toEqual(["alpha#12"]);
    // A number is also a word in a title.
    expect(numbers({}, "5")).toEqual(["alpha#12", "alpha#5"]);
    expect(numbers({}, "   ")).toHaveLength(7);
    expect(numbers({}, "nothing like this")).toEqual([]);
  });

  it("applies every filter at once", () => {
    expect(
      numbers(
        { state: "open", projects: [alpha.id], author: "grace", reviews: "awaitingYou" },
        "retry",
      ),
    ).toEqual(["alpha#2"]);
    expect(numbers({ state: "merged", author: "grace", reviews: "awaitingYou" })).toEqual([]);
  });
});

describe("reviewDecision", () => {
  it("is the forge's own decision when it has one", () => {
    const decided = pullRequest(1, {
      details: { review: "APPROVED", reviews: [{ login: "ken", state: "changesRequested" }] },
    });
    expect(reviewDecision(decided)).toBe("approved");
  });

  it("is worked out from the reviews where the repository has no review rule", () => {
    const of = (...states: ("approved" | "changesRequested" | "commented")[]) =>
      reviewDecision(
        pullRequest(1, {
          details: { review: "", reviews: states.map((state, i) => ({ login: `r${i}`, state })) },
        }),
      );
    expect(of("approved", "commented")).toBe("approved");
    expect(of("approved", "changesRequested"), "a request for changes outranks").toBe("changes");
    expect(of("commented")).toBeNull();
    expect(of()).toBeNull();
    expect(reviewDecision({ ...pullRequest(1), details: null })).toBeNull();
  });
});

describe("the filter bar's own questions", () => {
  it("lists each author once, in order", () => {
    expect(authorsOf(rows)).toEqual(["ada", "grace", "ken", "Linus"]);
  });

  it("knows whether anything is narrowing the list — the state tab is not", () => {
    expect(filtering(DEFAULT_FILTERS, "")).toBe(false);
    expect(filtering({ ...DEFAULT_FILTERS, state: "merged" }, "")).toBe(false);
    expect(filtering({ ...DEFAULT_FILTERS, author: "ada" }, "")).toBe(true);
    expect(filtering({ ...DEFAULT_FILTERS, projects: ["p"] }, "")).toBe(true);
    expect(filtering({ ...DEFAULT_FILTERS, reviews: "none" }, "")).toBe(true);
    expect(filtering(DEFAULT_FILTERS, " x ")).toBe(true);
  });
});

describe("readFilters", () => {
  const ids = [alpha.id, beta.id];

  it("gives back what was remembered", () => {
    const saved = { state: "merged", projects: [beta.id], author: "grace", reviews: "approved" };
    expect(readFilters(saved, ids)).toEqual(saved);
  });

  it("drops a project that is no longer open, rather than hide every row behind it", () => {
    expect(
      readFilters({ ...DEFAULT_FILTERS, projects: ["p-gone", alpha.id] }, ids).projects,
    ).toEqual([alpha.id]);
  });

  it("falls back to the defaults for anything it cannot read", () => {
    expect(readFilters(null, ids)).toEqual(DEFAULT_FILTERS);
    expect(readFilters("open", ids)).toEqual(DEFAULT_FILTERS);
    expect(
      readFilters({ state: "sideways", projects: "all", author: 7, reviews: ["x"] }, ids),
    ).toEqual(DEFAULT_FILTERS);
    expect(readFilters({ author: "" }, ids).author).toBeNull();
  });
});
