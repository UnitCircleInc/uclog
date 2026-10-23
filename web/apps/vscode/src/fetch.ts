// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Logdata service requests from the extension host (plan 03 §3): no CORS, and the token from
 * SecretStorage never enters the webview. The webview's resolver reads the reply as a Response.
 */

export interface FetchReply {
  status: number;
  headers: Record<string, string>;
  body: Uint8Array;
}

/** Only the logdata service (its URL setting) may be fetched for the webview. */
export function allowedUrl(url: string, serviceUrl: string): boolean {
  try {
    const u = new URL(url);
    const s = new URL(serviceUrl);
    return u.origin === s.origin && u.pathname.startsWith(`${s.pathname.replace(/\/+$/, '')}/logdata/`);
  } catch {
    return false;
  }
}

export async function hostFetch(
  url: string,
  headers: Record<string, string>,
  token: string | undefined,
  f: typeof fetch = fetch,
): Promise<FetchReply> {
  const h: Record<string, string> = { ...headers };
  if (token && !Object.keys(h).some((k) => k.toLowerCase() === 'authorization')) h.Authorization = `Bearer ${token}`;
  const r = await f(url, { headers: h, redirect: 'follow', signal: AbortSignal.timeout(30_000) });
  const out: Record<string, string> = {};
  r.headers.forEach((v, k) => {
    if (k === 'content-type' || k === 'x-request-id' || k === 'retry-after') out[k] = v;
  });
  return { status: r.status, headers: out, body: new Uint8Array(await r.arrayBuffer()) };
}
