export interface CompetitiveRankEntry {
  username: string;
  rank: number;
}

export interface CompetitiveRankSnapshot {
  tokens: CompetitiveRankEntry[];
  cost: CompetitiveRankEntry[];
}

function collectChangedUsernames(
  before: CompetitiveRankEntry[],
  after: CompetitiveRankEntry[]
): string[] {
  const beforeRanks = new Map(
    before.map(({ username, rank }) => [username, rank])
  );
  const afterRanks = new Map(
    after.map(({ username, rank }) => [username, rank])
  );
  const orderedUsernames = [
    ...after.map(({ username }) => username),
    ...before
      .map(({ username }) => username)
      .filter((username) => !afterRanks.has(username)),
  ];

  return orderedUsernames.filter(
    (username) => beforeRanks.get(username) !== afterRanks.get(username)
  );
}

export function getAffectedCompetitiveUsernames(
  before: CompetitiveRankSnapshot,
  after: CompetitiveRankSnapshot
): string[] {
  const affectedUsernames = [
    ...collectChangedUsernames(before.tokens, after.tokens),
    ...collectChangedUsernames(before.cost, after.cost),
  ];

  return Array.from(new Set(affectedUsernames));
}
