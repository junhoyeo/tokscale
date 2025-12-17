'use client';

import styled from '@emotion/styled';

const TokscaleLogo: React.FC = () => (
  <a href="https://tokscale.ai" style={{ textDecoration: 'none' }}>
    <LogoText>tokscale</LogoText>
  </a>
);

const LogoText = styled.span`
  font-size: 24px;
  font-weight: 700;
  color: #53d1f3;
  letter-spacing: -0.5px;
`;

export function Footer() {
  return (
    <Container>
      <TokscaleLogo />
      <CompanyInfo className="en">
        © 2025 Tokscale. All rights reserved.
      </CompanyInfo>
    </Container>
  );
}

const Container = styled.footer`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  width: 100%;
  padding: 36px 20px 120px;
  border-top: 1.5px solid black;

  @media screen and (max-width: 620px) {
    padding: 24px 20px 120px;
  }

  @media screen and (max-width: 340px) {
    padding: 20px 20px 120px;
  }
`;

const CompanyInfo = styled.p`
  font-weight: 400;
  font-size: 14px;
  line-height: 140%;
  text-align: center;
  line-break: keep-all;
  color: #778fad;

  strong {
    font-weight: bold;
  }
`;
