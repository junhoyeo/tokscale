CREATE TABLE "api_tokens" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"token" varchar(64) NOT NULL,
	"name" varchar(100) NOT NULL,
	"last_used_at" timestamp with time zone,
	"expires_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "api_tokens_token_unique" UNIQUE("token"),
	CONSTRAINT "api_tokens_user_name_unique" UNIQUE("user_id","name")
);
--> statement-breakpoint
CREATE TABLE "daily_breakdown" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"submission_id" uuid NOT NULL,
	"date" date NOT NULL,
	"tokens" bigint NOT NULL,
	"cost" numeric(10, 4) NOT NULL,
	"input_tokens" bigint NOT NULL,
	"output_tokens" bigint NOT NULL,
	"provider_breakdown" jsonb,
	"source_breakdown" jsonb,
	"model_breakdown" jsonb,
	CONSTRAINT "daily_breakdown_submission_date_unique" UNIQUE("submission_id","date")
);
--> statement-breakpoint
CREATE TABLE "device_codes" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"device_code" varchar(32) NOT NULL,
	"user_code" varchar(9) NOT NULL,
	"user_id" uuid,
	"device_name" varchar(100),
	"expires_at" timestamp with time zone NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "device_codes_device_code_unique" UNIQUE("device_code"),
	CONSTRAINT "device_codes_user_code_unique" UNIQUE("user_code")
);
--> statement-breakpoint
CREATE TABLE "sessions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"token" varchar(64) NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"source" varchar(10) DEFAULT 'web' NOT NULL,
	"user_agent" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "sessions_token_unique" UNIQUE("token")
);
--> statement-breakpoint
CREATE TABLE "submissions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid NOT NULL,
	"total_tokens" bigint NOT NULL,
	"total_cost" numeric(12, 4) NOT NULL,
	"input_tokens" bigint NOT NULL,
	"output_tokens" bigint NOT NULL,
	"cache_creation_tokens" bigint DEFAULT 0 NOT NULL,
	"cache_read_tokens" bigint DEFAULT 0 NOT NULL,
	"date_start" date NOT NULL,
	"date_end" date NOT NULL,
	"sources_used" text[] NOT NULL,
	"models_used" text[] NOT NULL,
	"status" varchar(20) DEFAULT 'verified' NOT NULL,
	"cli_version" varchar(20),
	"submission_hash" varchar(64),
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "submissions_user_id_unique" UNIQUE("user_id"),
	CONSTRAINT "submissions_user_hash_unique" UNIQUE("user_id", "submission_hash")
);
--> statement-breakpoint
CREATE TABLE "users" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"github_id" integer NOT NULL,
	"username" varchar(39) NOT NULL,
	"display_name" varchar(255),
	"avatar_url" text,
	"email" varchar(255),
	"is_admin" boolean DEFAULT false NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "users_github_id_unique" UNIQUE("github_id"),
	CONSTRAINT "users_username_unique" UNIQUE("username")
);
--> statement-breakpoint
ALTER TABLE "api_tokens" ADD CONSTRAINT "api_tokens_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "daily_breakdown" ADD CONSTRAINT "daily_breakdown_submission_id_submissions_id_fk" FOREIGN KEY ("submission_id") REFERENCES "public"."submissions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "device_codes" ADD CONSTRAINT "device_codes_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sessions" ADD CONSTRAINT "sessions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "submissions" ADD CONSTRAINT "submissions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "idx_api_tokens_token" ON "api_tokens" USING btree ("token");--> statement-breakpoint
CREATE INDEX "idx_api_tokens_user_id" ON "api_tokens" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "idx_daily_breakdown_submission_id" ON "daily_breakdown" USING btree ("submission_id");--> statement-breakpoint
CREATE INDEX "idx_daily_breakdown_date" ON "daily_breakdown" USING btree ("date");--> statement-breakpoint
CREATE INDEX "idx_device_codes_device_code" ON "device_codes" USING btree ("device_code");--> statement-breakpoint
CREATE INDEX "idx_device_codes_user_code" ON "device_codes" USING btree ("user_code");--> statement-breakpoint
CREATE INDEX "idx_device_codes_expires_at" ON "device_codes" USING btree ("expires_at");--> statement-breakpoint
CREATE INDEX "idx_sessions_token" ON "sessions" USING btree ("token");--> statement-breakpoint
CREATE INDEX "idx_sessions_user_id" ON "sessions" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "idx_sessions_expires_at" ON "sessions" USING btree ("expires_at");--> statement-breakpoint
CREATE INDEX "idx_submissions_user_id" ON "submissions" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "idx_submissions_status" ON "submissions" USING btree ("status");--> statement-breakpoint
CREATE INDEX "idx_submissions_total_tokens" ON "submissions" USING btree ("total_tokens");--> statement-breakpoint
CREATE INDEX "idx_submissions_created_at" ON "submissions" USING btree ("created_at");--> statement-breakpoint
CREATE INDEX "idx_submissions_date_range" ON "submissions" USING btree ("date_start","date_end");--> statement-breakpoint
CREATE INDEX "idx_users_username" ON "users" USING btree ("username");--> statement-breakpoint
CREATE INDEX "idx_users_github_id" ON "users" USING btree ("github_id");