CREATE TABLE "explore_cache_revision" (
	"id" integer PRIMARY KEY NOT NULL,
	"revision" bigint DEFAULT 0 NOT NULL,
	CONSTRAINT "explore_cache_revision_singleton" CHECK ("explore_cache_revision"."id" = 1)
);

--> statement-breakpoint
INSERT INTO "explore_cache_revision" ("id", "revision") VALUES (1, 0);
