ALTER TABLE "moderation_actions" DROP CONSTRAINT "moderation_actions_target_user_id_users_id_fk";
--> statement-breakpoint
ALTER TABLE "moderation_actions" DROP CONSTRAINT "moderation_actions_actor_user_id_users_id_fk";
--> statement-breakpoint
ALTER TABLE "moderation_actions" ALTER COLUMN "target_user_id" DROP NOT NULL;--> statement-breakpoint
ALTER TABLE "moderation_actions" ALTER COLUMN "actor_user_id" DROP NOT NULL;--> statement-breakpoint
ALTER TABLE "moderation_actions" ADD COLUMN "target_username" varchar(39);--> statement-breakpoint
ALTER TABLE "moderation_actions" ADD COLUMN "actor_username" varchar(39);--> statement-breakpoint
UPDATE "moderation_actions" AS action
SET "target_username" = users."username"
FROM "users"
WHERE action."target_user_id" = users."id";--> statement-breakpoint
UPDATE "moderation_actions" AS action
SET "actor_username" = users."username"
FROM "users"
WHERE action."actor_user_id" = users."id";--> statement-breakpoint
ALTER TABLE "moderation_actions" ALTER COLUMN "target_username" SET NOT NULL;--> statement-breakpoint
ALTER TABLE "moderation_actions" ALTER COLUMN "actor_username" SET NOT NULL;--> statement-breakpoint
ALTER TABLE "moderation_actions" ADD CONSTRAINT "moderation_actions_target_user_id_users_id_fk" FOREIGN KEY ("target_user_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_actions" ADD CONSTRAINT "moderation_actions_actor_user_id_users_id_fk" FOREIGN KEY ("actor_user_id") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;
