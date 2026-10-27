// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

// The one-file app (uclog-app-N.html, `vite build --mode single`): the element (its workers are
// inlined classic workers, which start in a file:// page) and the sample logdata in one file, for
// opening from disk. Pages opened from files share one origin, so its settings use their own
// storage key.

import '@uclog/element';
import sample from '../../../../spec/vectors/sample.logdata?url'; // lib mode inlines it (a data: URL)
import { startPage } from './page.ts';

startPage('uclog-app-file', sample);
