// Regenerates apps/mobile/lib/models/protocol.dart from
// packages/protocol/schema.json using quicktype.
//
//   npm run generate:dart         (from apps/desktop)
//
// `quicktype` is pinned as a devDependency of apps/desktop so codegen is
// reproducible. We prefer the locally installed binary and only fall back to
// `npx` (which would download an unpinned version) if it is missing.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const REPO_ROOT = path.resolve(__dirname, '..');
const SCHEMA_PATH = path.join(REPO_ROOT, 'packages', 'protocol', 'schema.json');
const DART_OUT_PATH = path.join(REPO_ROOT, 'apps', 'mobile', 'lib', 'models', 'protocol.dart');

// Resolve the pinned quicktype CLI without letting `npx` fetch a random
// version. `quicktype` is declared as a devDependency of apps/desktop, so
// that is the first place we look; the repo root is checked too in case the
// dependency is later hoisted into a root workspace.
const RESOLVE_ROOTS = [
  path.join(REPO_ROOT, 'apps', 'desktop'),
  REPO_ROOT,
];

function resolveQuicktype() {
  for (const root of RESOLVE_ROOTS) {
    for (const spec of ['quicktype/dist/index.js', 'quicktype-core/dist/cli.js']) {
      try {
        return require.resolve(spec, { paths: [root] });
      } catch (_) {
        /* try the next candidate */
      }
    }
  }
  return null;
}

console.log('Generating Dart models using quicktype...');

const cli = resolveQuicktype();
let command;
let args;

if (cli) {
  command = process.execPath;
  args = [
    cli,
    '--src-lang',
    'schema',
    '--lang',
    'dart',
    '--out',
    DART_OUT_PATH,
    SCHEMA_PATH,
  ];
  console.log(`Using pinned quicktype: ${cli}`);
} else {
  console.warn(
    'Pinned quicktype not found. Install dev dependencies with `npm install` ' +
      'in apps/desktop, or quicktype will be resolved unpinned via npx.'
  );
  command = 'npx';
  args = [
    '--yes',
    'quicktype',
    '--src-lang',
    'schema',
    '--lang',
    'dart',
    '--out',
    DART_OUT_PATH,
    SCHEMA_PATH,
  ];
}

try {
  if (!fs.existsSync(path.dirname(DART_OUT_PATH))) {
    throw new Error(
      `output directory does not exist: ${path.dirname(DART_OUT_PATH)}`
    );
  }
  execFileSync(command, args, { stdio: 'inherit', cwd: REPO_ROOT });
  console.log(`Dart generation successful -> ${path.relative(REPO_ROOT, DART_OUT_PATH)}`);
} catch (e) {
  if (e.status != null) {
    console.error(`Error generating Dart (exit ${e.status}).`);
  } else {
    console.error('Error generating Dart:', e.message);
  }
  process.exit(1);
}
