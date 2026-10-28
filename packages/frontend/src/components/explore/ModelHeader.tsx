"use client";

/**
 * Breadcrumb and identity header shared by the model pages (/models/[key],
 * with or without Tokscale usage) and the full catalog (/models/all).
 */
import type { ReactNode } from "react";
import Link from "next/link";
import styled from "styled-components";
import type { ModelRef } from "@/lib/explore";
import { companyPath } from "@/lib/seo/explore";
import { UNKNOWN_LAB_NAME } from "./catalog";
import { InlineLink, ModelMark, PageTitle } from "./ExploreUI";

const Breadcrumb = styled.nav`
  margin-bottom: 12px;

  ol {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--service-text-muted);
    font-size: 0.8125rem;
  }

  li:not(:last-child)::after {
    content: "/";
    margin-left: 6px;
    color: var(--service-border-strong);
  }

  @media (max-width: 640px) {
    ol {
      font-size: 0.875rem;
    }
  }
`;

const CrumbLink = styled(Link)`
  color: var(--service-text-muted);
  text-decoration: none;

  &:hover {
    color: var(--service-text);
    text-decoration: underline !important;
  }

  &:focus-visible {
    border-radius: 2px;
    outline: 2px solid var(--service-focus);
    outline-offset: 2px;
  }

  @media (pointer: coarse), (max-width: 640px) {
    display: inline-flex;
    min-height: 44px;
    align-items: center;
  }
`;

export interface Crumb {
  label: string;
  /** Omit for the current page. */
  href?: string;
}

/** Visible breadcrumb; the last item is the current page. */
export function ModelBreadcrumb({ items }: { items: readonly Crumb[] }) {
  return (
    <Breadcrumb aria-label="Breadcrumb">
      <ol>
        {items.map((item, index) =>
          item.href && index < items.length - 1 ? (
            <li key={`${item.href}-${item.label}`}>
              <CrumbLink href={item.href}>{item.label}</CrumbLink>
            </li>
          ) : (
            <li key={`current-${item.label}`} aria-current="page">
              {item.label}
            </li>
          ),
        )}
      </ol>
    </Breadcrumb>
  );
}

/* No outer margin: the page that places the identity owns the spacing around it. */
const Identity = styled.header`
  display: grid;
  min-width: 0;
  gap: 8px;
`;

const TitleRow = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 12px;
`;

const IdentityMeta = styled.p`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 0;
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;

  > span:not(:last-child)::after {
    content: "·";
    margin: 0 8px;
    color: var(--service-border-strong);
  }

  code {
    color: var(--service-text);
    font-size: 0.8125rem;
  }
`;

/** Breadcrumb trail for a model page: Models, the lab when it has a company page, the model. */
export function modelCrumbs(model: Pick<ModelRef, "name" | "lab" | "labSlug">): Crumb[] {
  return [
    { label: "Models", href: "/models" },
    ...(model.labSlug ? [{ label: model.lab ?? UNKNOWN_LAB_NAME, href: companyPath(model.labSlug) }] : []),
    { label: model.name },
  ];
}

/**
 * Model mark, name, lab and key. `model.labSlug` must already be a company
 * page slug (or null); the lab is linked only then. Extra facts go in `children`.
 */
export function ModelIdentity({ model, children }: { model: ModelRef; children?: ReactNode }) {
  const labName = model.lab ?? UNKNOWN_LAB_NAME;
  return (
    <Identity>
      <TitleRow>
        <ModelMark model={model} size={44} />
        <PageTitle>{model.name}</PageTitle>
      </TitleRow>
      <IdentityMeta>
        <span>{model.labSlug ? <InlineLink href={companyPath(model.labSlug)}>{labName}</InlineLink> : labName}</span>
        <span>
          Key <code>{model.key}</code>
        </span>
        {children}
      </IdentityMeta>
    </Identity>
  );
}
