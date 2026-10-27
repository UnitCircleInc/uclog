import { describe, expect, test } from 'vitest';
import { fileOriginHint, isLocalHost, localNetworkHint } from '../src/websocket.ts';

describe('local network (Chrome Local Network Access)', () => {
  test('local hosts', () => {
    for (const h of ['localhost', '127.0.0.1', '127.1.2.3', '[::1]', '10.0.0.5', '172.16.0.1', '172.31.255.1', '192.168.1.20', '169.254.1.1', 'labpc.local', 'fe80::1', 'fd12::1'])
      expect(isLocalHost(h), h).toBe(true);
    for (const h of ['uclog.unitcircle.ca', '8.8.8.8', '172.32.0.1', '192.169.0.1', '2001:db8::1', 'localhost.example.com'])
      expect(isLocalHost(h), h).toBe(false);
  });

  test('the hint is for a public page and a local hub only', () => {
    expect(localNetworkHint('ws://127.0.0.1:9000', 'uclog.unitcircle.ca')).toContain('Local network access');
    expect(localNetworkHint('ws://192.168.1.20:9000', 'uclog.unitcircle.ca')).toContain('Local network access');
    expect(localNetworkHint('ws://127.0.0.1:9000', 'localhost')).toBe('');
    expect(localNetworkHint('wss://hub.example.com', 'uclog.unitcircle.ca')).toBe('');
    expect(localNetworkHint('not a url', 'uclog.unitcircle.ca')).toBe('');
  });

  test('a page from a file that a hub refused', () => {
    expect(fileOriginHint(1008, 'null')).toContain('--allow-origin null');
    expect(fileOriginHint(1008, 'https://uclog.unitcircle.ca')).toBe('');
    expect(fileOriginHint(1006, 'null')).toBe('');
  });
});
