'use server';

export async function getStargazersCount(
  repo: string = 'junhoyeo/tokscale'
): Promise<number> {
  try {
    const res = await fetch(`https://api.github.com/repos/${repo}`, {
      next: { revalidate: 60 },
    });

    if (!res.ok) {
      return 0;
    }

    const data = await res.json();
    return data.stargazers_count ?? 0;
  } catch {
    return 0;
  }
}
