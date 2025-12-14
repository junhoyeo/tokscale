'use client';

import styled from '@emotion/styled';

const InevitableLogo: React.FC = () => (
  <a href="https://beta.inevitable.team/en" target="_blank" rel="noopener noreferrer">
    <img
      src="/inevitable-logo.svg"
      alt="Inevitable"
      style={{ height: 24 }}
    />
  </a>
);

const InlineBlock = styled.span`
  display: inline-block;
`;

export function Footer() {
  return (
    <Container>
      <InevitableLogo />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <CompanyInfo className="en">
          © 2023-2025{' '}
          <InlineBlock className="allow-select">
            Strokecompany Co., Ltd.
          </InlineBlock>{' '}
          <InlineBlock>All rights reserved.</InlineBlock>
        </CompanyInfo>
        <CompanyInfo className="en">
          <a href="mailto:hello@strokecompany.io" style={{ color: '#53d1f3' }}>
            hello@strokecompany.io
          </a>
        </CompanyInfo>
      </div>
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
