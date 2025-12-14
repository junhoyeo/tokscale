"use client";

interface SkeletonProps {
  className?: string;
}

export function Skeleton({ className = "" }: SkeletonProps) {
  return (
    <div
      className={`animate-pulse rounded ${className}`}
      style={{ backgroundColor: "#262627" }}
    />
  );
}

export function LeaderboardSkeleton() {
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-10">
        {[...Array(4)].map((_, i) => (
          <div
            key={i}
            className="rounded-xl border p-4"
            style={{ backgroundColor: "#141415", borderColor: "#262627" }}
          >
            <Skeleton className="h-4 w-20 mb-2" />
            <Skeleton className="h-8 w-24" />
          </div>
        ))}
      </div>

      <div
        className="rounded-2xl border overflow-hidden"
        style={{ backgroundColor: "#141415", borderColor: "#262627" }}
      >
        <div
          className="border-b px-6 py-3"
          style={{ backgroundColor: "#1F1F20", borderColor: "#262627" }}
        >
          <div className="flex gap-6">
            <Skeleton className="h-4 w-12" />
            <Skeleton className="h-4 w-24" />
            <Skeleton className="h-4 w-16 ml-auto" />
            <Skeleton className="h-4 w-16" />
          </div>
        </div>
        {[...Array(10)].map((_, i) => (
          <div
            key={i}
            className="px-6 py-4 border-b last:border-0"
            style={{ borderColor: "#262627" }}
          >
            <div className="flex items-center gap-6">
              <Skeleton className="h-6 w-8" />
              <div className="flex items-center gap-3">
                <Skeleton className="h-10 w-10 rounded-full" />
                <div>
                  <Skeleton className="h-4 w-32 mb-1" />
                  <Skeleton className="h-3 w-20" />
                </div>
              </div>
              <Skeleton className="h-5 w-16 ml-auto" />
              <Skeleton className="h-5 w-14" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function ProfileSkeleton() {
  return (
    <div className="space-y-8">
      <div className="flex items-start gap-6 mb-8">
        <Skeleton className="w-24 h-24 rounded-2xl" />
        <div className="flex-1">
          <div className="flex items-center gap-3 mb-2">
            <Skeleton className="h-8 w-48" />
            <Skeleton className="h-6 w-12 rounded-lg" />
          </div>
          <Skeleton className="h-4 w-24 mb-3" />
          <div className="flex gap-2">
            <Skeleton className="h-6 w-16 rounded-lg" />
            <Skeleton className="h-6 w-20 rounded-lg" />
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {[...Array(4)].map((_, i) => (
          <div
            key={i}
            className="rounded-xl border p-4"
            style={{ backgroundColor: "#141415", borderColor: "#262627" }}
          >
            <Skeleton className="h-4 w-20 mb-2" />
            <Skeleton className="h-8 w-24" />
          </div>
        ))}
      </div>

      <div
        className="rounded-2xl border p-6"
        style={{ backgroundColor: "#141415", borderColor: "#262627" }}
      >
        <Skeleton className="h-6 w-36 mb-4" />
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          {[...Array(4)].map((_, i) => (
            <div key={i}>
              <Skeleton className="h-4 w-16 mb-1" />
              <Skeleton className="h-6 w-20" />
            </div>
          ))}
        </div>
      </div>

      <div
        className="rounded-2xl border p-6"
        style={{ backgroundColor: "#141415", borderColor: "#262627" }}
      >
        <Skeleton className="h-6 w-24 mb-4" />
        <Skeleton className="h-40 w-full rounded-lg" />
      </div>
    </div>
  );
}

export function StatCardSkeleton() {
  return (
    <div
      className="rounded-xl border p-4"
      style={{ backgroundColor: "#141415", borderColor: "#262627" }}
    >
      <Skeleton className="h-4 w-20 mb-2" />
      <Skeleton className="h-8 w-24" />
    </div>
  );
}

export function TableRowSkeleton() {
  return (
    <div
      className="px-6 py-4 border-b"
      style={{ borderColor: "#262627" }}
    >
      <div className="flex items-center gap-6">
        <Skeleton className="h-6 w-8" />
        <div className="flex items-center gap-3">
          <Skeleton className="h-10 w-10 rounded-full" />
          <div>
            <Skeleton className="h-4 w-32 mb-1" />
            <Skeleton className="h-3 w-20" />
          </div>
        </div>
        <Skeleton className="h-5 w-16 ml-auto" />
        <Skeleton className="h-5 w-14" />
      </div>
    </div>
  );
}
