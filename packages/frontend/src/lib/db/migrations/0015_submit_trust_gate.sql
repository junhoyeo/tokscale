CREATE TABLE "submission_reviews" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"submission_hash" varchar(64) NOT NULL,
	"trust_state" varchar(20) DEFAULT 'review_required' NOT NULL,
	"competitive_write_applied" boolean DEFAULT false NOT NULL,
	"reason_codes" text[] DEFAULT ARRAY[]::text[] NOT NULL,
	"payload" jsonb NOT NULL,
	"total_tokens" bigint NOT NULL,
	"total_cost" numeric(18, 4) NOT NULL,
	"active_days" integer NOT NULL,
	"date_start" date NOT NULL,
	"date_end" date NOT NULL,
	"sources_used" text[] NOT NULL,
	"models_used" text[] NOT NULL,
	"cli_version" varchar(20),
	"schema_version" integer DEFAULT 0 NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "submission_reviews_trust_state_check" CHECK ("submission_reviews"."trust_state" in ('trusted', 'review_required', 'rejected')),
	CONSTRAINT "submission_reviews_total_tokens_check" CHECK ("submission_reviews"."total_tokens" >= 0),
	CONSTRAINT "submission_reviews_total_cost_check" CHECK ("submission_reviews"."total_cost" >= 0),
	CONSTRAINT "submission_reviews_active_days_check" CHECK ("submission_reviews"."active_days" >= 0),
	CONSTRAINT "submission_reviews_schema_version_check" CHECK ("submission_reviews"."schema_version" >= 0),
	CONSTRAINT "submission_reviews_date_range_check" CHECK ("submission_reviews"."date_start" <= "submission_reviews"."date_end")
);
--> statement-breakpoint
ALTER TABLE "submission_reviews" ADD CONSTRAINT "submission_reviews_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "submission_reviews_pending_user_hash_unique" ON "submission_reviews" USING btree ("user_id","submission_hash") WHERE "submission_reviews"."trust_state" = 'review_required';--> statement-breakpoint
CREATE INDEX "idx_submission_reviews_user_id" ON "submission_reviews" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "idx_submission_reviews_trust_state" ON "submission_reviews" USING btree ("trust_state");--> statement-breakpoint
CREATE INDEX "idx_submission_reviews_created_at" ON "submission_reviews" USING btree ("created_at");
