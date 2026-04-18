import { describe, expect, it } from "vitest";
import { getAffectedCompetitiveUsernames } from "../../src/lib/leaderboard/competitiveRankChanges";

describe("getAffectedCompetitiveUsernames", () => {
  it("returns every username whose all-time rank changes across tokens or cost", () => {
    const affectedUsernames = getAffectedCompetitiveUsernames(
      {
        tokens: [
          { username: "bob", rank: 1 },
          { username: "carol", rank: 2 },
        ],
        cost: [
          { username: "carol", rank: 1 },
          { username: "bob", rank: 2 },
        ],
      },
      {
        tokens: [
          { username: "alice", rank: 1 },
          { username: "bob", rank: 2 },
          { username: "carol", rank: 3 },
        ],
        cost: [
          { username: "bob", rank: 1 },
          { username: "alice", rank: 2 },
          { username: "carol", rank: 3 },
        ],
      }
    );

    expect(new Set(affectedUsernames)).toEqual(new Set(["alice", "bob", "carol"]));
  });

  it("ignores users whose ranks stay unchanged", () => {
    expect(
      getAffectedCompetitiveUsernames(
        {
          tokens: [
            { username: "alice", rank: 1 },
            { username: "bob", rank: 2 },
          ],
          cost: [
            { username: "alice", rank: 1 },
            { username: "bob", rank: 2 },
          ],
        },
        {
          tokens: [
            { username: "alice", rank: 1 },
            { username: "bob", rank: 2 },
          ],
          cost: [
            { username: "alice", rank: 1 },
            { username: "bob", rank: 2 },
          ],
        }
      )
    ).toEqual([]);
  });

  it("does not treat reordered ties as rank changes", () => {
    expect(
      getAffectedCompetitiveUsernames(
        {
          tokens: [
            { username: "alice", rank: 1 },
            { username: "bob", rank: 1 },
          ],
          cost: [],
        },
        {
          tokens: [
            { username: "bob", rank: 1 },
            { username: "alice", rank: 1 },
          ],
          cost: [],
        }
      )
    ).toEqual([]);
  });
});
