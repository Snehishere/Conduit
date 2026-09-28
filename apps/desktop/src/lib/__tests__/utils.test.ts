import { describe, it, expect } from 'vitest';
import { cn, formatTime, getDeviceIconComponent } from '../utils';

describe('cn', () => {
  it('merges class names', () => {
    expect(cn('a', 'b')).toBe('a b');
  });

  it('handles conditional classes', () => {
    expect(cn('a', false && 'b', 'c')).toBe('a c');
  });

  it('handles undefined and null', () => {
    expect(cn('a', undefined, null, 'b')).toBe('a b');
  });

  it('handles empty input', () => {
    expect(cn()).toBe('');
  });

  it('handles Tailwind merge conflicts', () => {
    expect(cn('px-2 py-2', 'px-4')).toBe('py-2 px-4');
  });
});

describe('formatTime', () => {
  it('returns "now" for recent timestamps', () => {
    const now = Math.floor(Date.now() / 1000);
    expect(formatTime(now)).toBe('now');
  });

  it('returns minutes for timestamps within an hour', () => {
    const fiveMinAgo = Math.floor(Date.now() / 1000) - 300;
    expect(formatTime(fiveMinAgo)).toBe('5m');
  });

  it('returns hours for timestamps within a day', () => {
    const twoHrAgo = Math.floor(Date.now() / 1000) - 7200;
    expect(formatTime(twoHrAgo)).toBe('2h');
  });

  it('returns days for timestamps within a week', () => {
    const threeDaysAgo = Math.floor(Date.now() / 1000) - 259200;
    expect(formatTime(threeDaysAgo)).toBe('3d');
  });
});

describe('getDeviceIconComponent', () => {
  it('returns Monitor for desktop', () => {
    const icon = getDeviceIconComponent('desktop');
    expect(icon.displayName).toBe('Monitor');
  });

  it('returns Smartphone for phone', () => {
    const icon = getDeviceIconComponent('phone');
    expect(icon.displayName).toBe('Smartphone');
  });

  it('returns Tablet for tablet', () => {
    const icon = getDeviceIconComponent('tablet');
    expect(icon.displayName).toBe('Tablet');
  });

  it('returns Watch for watch', () => {
    const icon = getDeviceIconComponent('watch');
    expect(icon.displayName).toBe('Watch');
  });

  it('returns Tv for tv', () => {
    const icon = getDeviceIconComponent('tv');
    expect(icon.displayName).toBe('Tv');
  });

  it('returns Smartphone as default', () => {
    const icon = getDeviceIconComponent('unknown');
    expect(icon.displayName).toBe('Smartphone');
  });
});
