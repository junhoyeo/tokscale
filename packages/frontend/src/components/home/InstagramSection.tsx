"use client";

import styled, { css } from "styled-components";
import { TOKSCALE_INSTAGRAM_HANDLE, TOKSCALE_INSTAGRAM_URL } from "@/lib/community";
import { Container, Eyebrow, focusRing, Lead, Section, SectionHeader, SectionTitle, TextLink } from "./primitives";

export interface InstagramPost {
  href: string;
  /** Vendored card image (public/assets/v2/instagram, see SOURCES.json there). */
  image: string;
  alt: string;
  /** The post's first line. */
  caption: string;
}

/** Posts from @tokscaleai shown on the home page, refreshed by hand. */
export const INSTAGRAM_POSTS: InstagramPost[] = [
  {
    href: "https://www.instagram.com/p/DemjVOaElI9/",
    image: "/assets/v2/instagram/DemjVOaElI9.webp",
    alt: "Card: Tracking a trillion tokens worldwide.",
    caption: "You opened your laptop for one small fix. Now the sky outside is turning blue.",
  },
];

export function InstagramSection({ posts = INSTAGRAM_POSTS }: { posts?: InstagramPost[] }) {
  return (
    <Section aria-labelledby="home-instagram-title">
      <Container>
        <Layout $single={posts.length === 1}>
          <SectionHeader>
            <Eyebrow>Instagram</Eyebrow>
            <SectionTitle id="home-instagram-title">Tokscale data, in your feed.</SectionTitle>
            <Lead>Cards on how people build with AI, drawn from Tokscale data, and on the money behind AI.</Lead>
            <TextLink href={TOKSCALE_INSTAGRAM_URL} target="_blank" rel="noopener noreferrer">
              Follow {TOKSCALE_INSTAGRAM_HANDLE} <span aria-hidden="true">↗</span>
            </TextLink>
          </SectionHeader>
          <Posts $single={posts.length === 1}>
            {posts.map((post) => (
              <li key={post.href}>
                <PostLink href={post.href} target="_blank" rel="noopener noreferrer">
                  {/* eslint-disable-next-line @next/next/no-img-element -- small vendored card images */}
                  <img src={post.image} alt={post.alt} width={540} height={720} loading="lazy" decoding="async" />
                  <Caption>{post.caption}</Caption>
                </PostLink>
              </li>
            ))}
          </Posts>
        </Layout>
      </Container>
    </Section>
  );
}

/* One post sits beside the copy, like the featured profile; more posts form a grid under it. */
const Layout = styled.div<{ $single: boolean }>`
  display: grid;
  gap: 40px;

  ${({ $single }) =>
    $single &&
    css`
      grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
      align-items: center;
      gap: 64px;

      @media (max-width: 960px) {
        grid-template-columns: minmax(0, 1fr);
        gap: 32px;
      }
    `}
`;

const Posts = styled.ul<{ $single: boolean }>`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 20px;
  margin: 0;
  padding: 0;
  list-style: none;

  @media (max-width: 900px) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 16px;
  }

  ${({ $single }) =>
    $single &&
    css`
      grid-template-columns: minmax(0, 1fr);
      width: min(100%, 360px);
      justify-self: center;

      @media (max-width: 900px) {
        grid-template-columns: minmax(0, 1fr);
      }
    `}
`;

const PostLink = styled.a`
  display: flex;
  flex-direction: column;
  gap: 12px;
  border-radius: 12px;
  color: inherit;
  text-decoration: none;
  ${focusRing}

  img {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 3 / 4;
    object-fit: cover;
    border-radius: 12px;
    box-shadow: 0 0 0 1px var(--v2-rule-strong);
    transition: transform 160ms ease, box-shadow 160ms ease;
  }

  &:hover img {
    box-shadow: 0 0 0 1px var(--v2-accent);
  }

  @media (prefers-reduced-motion: no-preference) {
    &:hover img {
      transform: translateY(-2px);
    }
  }
`;

/* Two lines at most; the full text is on Instagram. */
const Caption = styled.span`
  display: -webkit-box;
  overflow: hidden;
  color: var(--v2-text-muted);
  font-size: 14px;
  line-height: 1.5;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
`;
