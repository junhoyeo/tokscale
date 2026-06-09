import { sql } from "drizzle-orm";
import type { SQL } from "drizzle-orm";
import type { PgColumn } from "drizzle-orm/pg-core";

export const USERS_USERNAME_LOWER_UNIQUE_INDEX = "users_username_lower_unique";

export function usernameLowerExpression(column: PgColumn): SQL {
  return sql`lower(${column})`;
}
