import React from 'react';

interface AboutSectionProps {
  appVersion: string;
}

const AboutSection: React.FC<AboutSectionProps> = ({ appVersion }) => (
  <div className="space-y-5">
    <div className="text-center py-4">
      <div className="text-[18px] font-semibold mb-1 text-ink-1">Conduit</div>
      <div className="text-[12px] text-ink-3">Version {appVersion}</div>
    </div>
  </div>
);

export default AboutSection;
