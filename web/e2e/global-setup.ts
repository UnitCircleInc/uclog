// Before every run: build @uclog/element (packages/element/dist). The embedding tests load its
// bundles, and the web app's production builds (offline, hosted) use it as published.

import { execSync } from 'node:child_process';
import { join } from 'node:path';

export default function globalSetup() {
  execSync('npx vite build && npx vite build --mode iife && npx vite build --mode workers', {
    cwd: join(import.meta.dirname, '../packages/element'),
    stdio: 'ignore',
  });
}
