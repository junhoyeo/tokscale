"use client";

import styled from "styled-components";
import { TOKSCALE_INSTAGRAM_HANDLE, TOKSCALE_INSTAGRAM_URL } from "@/lib/community";
import { Container, Lead, Section, SectionTitle, TextLink, focusRing } from "./primitives";
import { Reveal } from "./Reveal";

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
          <Copy>
            <Reveal>
              <SectionTitle id="home-instagram-title">Tokscale data, in your feed.</SectionTitle>
            </Reveal>
            <Reveal delay={70}>
              <Lead>Cards on how people build with AI, drawn from Tokscale data, and on the money behind AI.</Lead>
            </Reveal>
            <Reveal delay={140}>
              <TextLink href={TOKSCALE_INSTAGRAM_URL} target="_blank" rel="noopener noreferrer">
                Follow {TOKSCALE_INSTAGRAM_HANDLE} <span aria-hidden="true">↗</span>
              </TextLink>
            </Reveal>
          </Copy>
          <Posts $single={posts.length === 1}>
            {posts.map((post, index) => (
              <Reveal as="li" key={post.href} delay={80 + Math.min(index, 3) * 60}>
                <PostLink href={post.href} target="_blank" rel="noopener noreferrer">
                  {/* eslint-disable-next-line @next/next/no-img-element -- small vendored card images */}
                  <img src={post.image} alt={post.alt} width={540} height={720} loading="lazy" decoding="async" />
                  <Caption>{post.caption}</Caption>
                </PostLink>
              </Reveal>
            ))}
          </Posts>
        </Layout>
      </Container>
    </Section>
  );
}

/* Copy on the left, the post card on the right — the same split as the hero,
   with the picture at its own 3:4 crop. */
const Layout = styled.div<{ $single: boolean }>`
  display: grid;
  grid-template-columns: minmax(0, 6fr) minmax(0, 5fr);
  gap: 48px;
  align-items: center;

  @media (max-width: 960px) {
    grid-template-columns: minmax(0, 1fr);
    gap: 32px;
  }
`;

const Copy = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
`;

const Posts = styled.ul<{ $single: boolean }>`
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 16px;
  margin: 0;
  padding: 0;
  list-style: none;

  ${({ $single }) =>
    $single &&
    `
      grid-template-columns: minmax(0, 1fr);
      width: min(100%, 460px);
      justify-self: end;
    `}

  @media (max-width: 960px) {
    justify-self: start;
    width: min(100%, 460px);
  }
`;

const PostLink = styled.a`
  display: flex;
  flex-direction: column;
  gap: 12px;
  border-radius: var(--radius, 2px);
  color: inherit;
  text-decoration: none;
  ${focusRing}

  img {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 3 / 4;
    object-fit: cover;
    border-radius: var(--radius, 2px);
    background: var(--v2-surface);
  }
`;

/* Two lines at most; the full text is on Instagram. */
const Caption = styled.span`
  display: -webkit-box;
  overflow: hidden;
  color: var(--v2-text-muted);
  font-size: 15px;
  line-height: 1.5;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
`;
