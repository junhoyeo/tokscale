CREATE TABLE "group_invite_redemptions" (
	"invite_id" uuid NOT NULL,
	"user_id" uuid NOT NULL,
	"redeemed_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "group_invite_redemptions_invite_id_user_id_pk" PRIMARY KEY("invite_id","user_id")
);
--> statement-breakpoint
ALTER TABLE "group_invites" ADD COLUMN "max_uses" integer DEFAULT 1 NOT NULL;--> statement-breakpoint
ALTER TABLE "group_invites" ADD COLUMN "use_count" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "group_invite_redemptions" ADD CONSTRAINT "group_invite_redemptions_invite_id_group_invites_id_fk" FOREIGN KEY ("invite_id") REFERENCES "public"."group_invites"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "group_invite_redemptions" ADD CONSTRAINT "group_invite_redemptions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_group_invite_redemptions_user" ON "group_invite_redemptions" USING btree ("user_id");--> statement-breakpoint
ALTER TABLE "group_invites" ADD CONSTRAINT "group_invites_use_limits" CHECK ("group_invites"."max_uses" BETWEEN 1 AND 1000 AND "group_invites"."use_count" BETWEEN 0 AND "group_invites"."max_uses");--> statement-breakpoint
ALTER TABLE "group_invites" ADD CONSTRAINT "group_invites_target_single_use" CHECK (("group_invites"."invited_username_normalized" IS NULL AND "group_invites"."invited_user_id" IS NULL) OR "group_invites"."max_uses" = 1);
--> statement-breakpoint
UPDATE "group_invites" SET "use_count" = 1 WHERE "status" = 'accepted';
