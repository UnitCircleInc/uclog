// The Custom Elements Manifest for <uc-log> (`npx cem analyze`): packages/element/dist/
// custom-elements.json, shipped in the package (editors read it) and rendered as the docs'
// reference page by website/build.py. The API is documented in JSDoc on the class
// (packages/ui/src/uc-log.ts); `@internal` leaves a member out.

/** Lit's internal state (`{state: true}` in `static properties`) is not an attribute or part of
 * the API; and events fired through a helper (`this.fire(type, …)`) are found as one named
 * `type`: the real ones are the class's `@fires`. */
function litState() {
  const state = new Set();
  return {
    name: 'uclog-lit-state',
    analyzePhase({ ts, node }) {
      if (!ts.isPropertyDeclaration(node) || node.name.getText() !== 'properties') return;
      if (!node.initializer || !ts.isObjectLiteralExpression(node.initializer)) return;
      for (const p of node.initializer.properties) {
        if (ts.isPropertyAssignment(p) && /\bstate\s*:\s*true\b/.test(p.initializer.getText())) state.add(p.name.getText());
      }
    },
    packageLinkPhase({ customElementsManifest }) {
      for (const m of customElementsManifest.modules) {
        for (const d of m.declarations ?? []) {
          if (d.attributes) d.attributes = d.attributes.filter((a) => !state.has(a.fieldName ?? a.name));
          if (d.members) d.members = d.members.filter((x) => !state.has(x.name));
          if (d.events) d.events = d.events.filter((e) => e.name !== 'type');
        }
      }
    },
  };
}

export default {
  globs: ['packages/ui/src/uc-log.ts'],
  outdir: 'packages/element/dist',
  litelement: true,
  plugins: [litState()],
};
