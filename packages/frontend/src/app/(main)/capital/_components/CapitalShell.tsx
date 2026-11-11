import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { SubNav, type CapitalSection } from "./SubNav";
import styles from "./capital.module.css";

/** Page frame for the capital section. Server-rendered. `section` marks the active sub-navigation item. */
export function CapitalShell({ children, section }: { children: React.ReactNode; section?: CapitalSection }) {
  return (
    <div className="service-page-shell">
      <Navigation />
      <main className="service-main" id="main-content">
        <div className={styles.page}>
          <SubNav active={section} />
          {children}
        </div>
      </main>
      <PublicFooter />
    </div>
  );
}
