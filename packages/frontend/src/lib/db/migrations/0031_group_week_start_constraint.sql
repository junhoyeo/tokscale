ALTER TABLE "groups" ADD CONSTRAINT "groups_week_starts_on_check" CHECK ("groups"."week_starts_on" BETWEEN 0 AND 6) NOT VALID;--> statement-breakpoint
ALTER TABLE "groups" VALIDATE CONSTRAINT "groups_week_starts_on_check";
