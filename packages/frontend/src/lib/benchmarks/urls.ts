import { getPublicOrigin } from "@/lib/seo/urls";

/** Path builders for the benchmark pages and the full model catalog. */
export const benchmarksPath = () => "/benchmarks";
export const benchmarkPath = (slug: string) => `/benchmarks/${encodeURIComponent(slug)}`;
export const modelsAllPath = () => "/models/all";

export const benchmarksUrl = () => `${getPublicOrigin()}${benchmarksPath()}`;
export const benchmarkUrl = (slug: string) => `${getPublicOrigin()}${benchmarkPath(slug)}`;
export const modelsAllUrl = () => `${getPublicOrigin()}${modelsAllPath()}`;
