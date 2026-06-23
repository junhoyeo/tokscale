-- Groups feature: scoped leaderboards, membership roles, and hashed invite links.

CREATE TABLE IF NOT EXISTS "groups" (
  "id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "name" varchar(100) NOT NULL,
  "slug" varchar(100) NOT NULL,
  "description" text,
  "avatar_url" text,
  "is_public" boolean DEFAULT true NOT NULL,
  "created_by" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  CONSTRAINT "groups_slug_unique" UNIQUE("slug")
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS "group_members" (
  "id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "group_id" uuid NOT NULL,
  "user_id" uuid NOT NULL,
  "role" varchar(10) DEFAULT 'member' NOT NULL,
  "invited_by" uuid,
  "joined_at" timestamp with time zone DEFAULT now() NOT NULL,
  CONSTRAINT "group_members_group_user_unique" UNIQUE("group_id", "user_id")
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS "group_invites" (
  "id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "group_id" uuid NOT NULL,
  "invited_username" varchar(39),
  "invited_username_normalized" varchar(39),
  "invited_user_id" uuid,
  "invited_by" uuid NOT NULL,
  "role" varchar(10) DEFAULT 'member' NOT NULL,
  "status" varchar(10) DEFAULT 'pending' NOT NULL,
  "token_hash" varchar(64) NOT NULL,
  "expires_at" timestamp with time zone NOT NULL,
  "accepted_at" timestamp with time zone,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  CONSTRAINT "group_invites_token_hash_unique" UNIQUE("token_hash")
);
--> statement-breakpoint
ALTER TABLE "groups" ADD CONSTRAINT "groups_created_by_users_id_fk" FOREIGN KEY ("created_by") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_members" ADD CONSTRAINT "group_members_group_id_groups_id_fk" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_members" ADD CONSTRAINT "group_members_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_members" ADD CONSTRAINT "group_members_invited_by_users_id_fk" FOREIGN KEY ("invited_by") REFERENCES "public"."users"("id") ON DELETE set null ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_invites" ADD CONSTRAINT "group_invites_group_id_groups_id_fk" FOREIGN KEY ("group_id") REFERENCES "public"."groups"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_invites" ADD CONSTRAINT "group_invites_invited_user_id_users_id_fk" FOREIGN KEY ("invited_user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "group_invites" ADD CONSTRAINT "group_invites_invited_by_users_id_fk" FOREIGN KEY ("invited_by") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_groups_created_by" ON "groups" USING btree ("created_by");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_groups_visibility_updated" ON "groups" USING btree ("is_public", "updated_at");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_members_user_id" ON "group_members" USING btree ("user_id");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_invites_group_status" ON "group_invites" USING btree ("group_id", "status");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_invites_invited_user_status" ON "group_invites" USING btree ("invited_user_id", "status");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_invites_invited_username_status" ON "group_invites" USING btree ("invited_username_normalized", "status");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_group_invites_expires_at" ON "group_invites" USING btree ("expires_at");
