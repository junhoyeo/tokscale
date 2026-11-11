import { track } from "@vercel/analytics";

export type AnalyticsEventMap = {
  install_command_copied: {
    location: "home" | "tracker_hero" | "tracker_footer" | "leaderboard";
    command: "install" | "login" | "submit" | "api_url";
  };
  readme_snippet_copied: { location: "tracker_embed" };
  sign_in_clicked: {
    location: "desktop_nav" | "mobile_nav" | "groups_browser" | "device" | "join_group";
  };
  outbound_link_clicked: {
    target: "github" | "discord" | "x" | "instagram" | "sponsor";
    location:
      | "public_footer"
      | "tracker_docs"
      | "tracker_source"
      | "home_instagram_follow"
      | "home_instagram_post";
  };
  leaderboard_view_selected: { view: "humans" | "models" | "agents" | "groups" };
  profile_shared: undefined;
  profile_embed_copied: { format: "url" | "html" | "markdown" };
  local_viewer_loaded: { source: "paste" | "file" | "drop" | "sample" };
};

export type LocalViewerSource = AnalyticsEventMap["local_viewer_loaded"]["source"];

export function trackEvent<Name extends keyof AnalyticsEventMap>(
  name: Name,
  ...args: AnalyticsEventMap[Name] extends undefined
    ? []
    : [properties: AnalyticsEventMap[Name]]
): void {
  track(name, args[0]);
}
