// Regenerates apps/desktop/src/types/websocket.ts from
// packages/protocol/schema.json.
//
//   npm run generate:types        (from apps/desktop)
//
// The schema is the wire contract; this script is a pure projection of it.
// It deliberately fails loudly rather than guessing, because a silently
// dropped constraint here becomes a compile-time hole in the desktop app.

const fs = require('fs');
const path = require('path');

const SCHEMA_PATH = path.resolve(
  __dirname,
  '..',
  'packages',
  'protocol',
  'schema.json'
);
const TS_OUT_PATH = path.resolve(
  __dirname,
  '..',
  'apps',
  'desktop',
  'src',
  'types',
  'websocket.ts'
);

const schema = JSON.parse(fs.readFileSync(SCHEMA_PATH, 'utf8'));

/** Resolve a local `$ref` such as `#/definitions/SmsThread` to its name. */
function refName(ref) {
  const parts = String(ref).split('/');
  return parts[parts.length - 1];
}

function literal(value) {
  return typeof value === 'string' ? `'${value}'` : JSON.stringify(value);
}

function primitive(name) {
  switch (name) {
    case 'string':
      return 'string';
    case 'integer':
    case 'number':
      return 'number';
    case 'boolean':
      return 'boolean';
    case 'null':
      return 'null';
    case 'array':
      return 'unknown[]';
    case 'object':
      // Deliberately `any`: several branches carry free-form payloads
      // (`relay_route.payload`, `status.device_info` payloads) that the
      // desktop passes straight through. Tightening this breaks callers.
      return 'any';
    default:
      return 'unknown';
  }
}

/**
 * Map a JSON Schema property to a TypeScript type.
 *
 * Handles `$ref`, `const`, `enum`, `type` arrays (e.g. `["string","null"]`),
 * `allOf` (intersection) and `anyOf`/`oneOf` (union).
 */
function typeOf(def) {
  if (!def || typeof def !== 'object') return 'unknown';
  if (Object.prototype.hasOwnProperty.call(def, 'const')) {
    return literal(def.const);
  }
  if (Array.isArray(def.enum)) {
    return def.enum.map(literal).join(' | ');
  }
  if (def.$ref) return refName(def.$ref);
  for (const keyword of ['anyOf', 'oneOf']) {
    if (Array.isArray(def[keyword])) {
      return def[keyword].map(typeOf).join(' | ');
    }
  }
  if (Array.isArray(def.allOf)) {
    const parts = def.allOf.map(typeOf).filter((t) => t !== 'unknown');
    return parts.length > 0 ? parts.join(' & ') : 'unknown';
  }
  if (Array.isArray(def.type)) {
    return def.type.map(primitive).join(' | ');
  }
  switch (def.type) {
    case 'string':
    case 'integer':
    case 'number':
    case 'boolean':
    case 'object':
      return primitive(def.type);
    case 'array':
      return `Array<${def.items ? typeOf(def.items) : 'any'}>`;
    default:
      return 'unknown';
  }
}

/**
 * Collect every required-property constraint a subschema imposes.
 *
 * `required` and `allOf` are conjunctive: each listed property must be
 * present, so they merge into a single flat set. `anyOf` / `oneOf` are
 * alternatives: each subschema that declares `required` contributes its own
 * group, and satisfying any single group is sufficient.
 *
 * Ignoring the combinators here is what previously made `Automation Delete`
 * generate with both `rule_id` and `id` optional, silently discarding the
 * "one of rule_id | id" constraint the schema actually encodes.
 */
function requiredConstraints(obj) {
  const always = new Set(Array.isArray(obj.required) ? obj.required : []);
  const groups = [];

  for (const sub of Array.isArray(obj.allOf) ? obj.allOf : []) {
    for (const name of Array.isArray(sub.required) ? sub.required : []) {
      always.add(name);
    }
  }
  for (const keyword of ['anyOf', 'oneOf']) {
    for (const sub of Array.isArray(obj[keyword]) ? obj[keyword] : []) {
      if (Array.isArray(sub.required) && sub.required.length > 0) {
        groups.push(new Set(sub.required));
      }
    }
  }
  return { always, groups };
}

/** Render `properties` at the given indentation, honouring `required`. */
function renderProperties(props, required, indent) {
  return Object.entries(props).map(
    ([name, def]) =>
      `${indent}${name}${required.has(name) ? '' : '?'}: ${typeOf(def)};`
  );
}

/**
 * Render a schema object as either an `interface` (no alternative-required
 * constraints) or a `type` alias intersecting a base shape with the union of
 * its alternatives.
 */
function renderObject(name, def, indent = '') {
  const { always, groups } = requiredConstraints(def);
  const props = def.properties || {};
  const base = renderProperties(props, always, `${indent}  `);

  // A single alternative is just another conjunction.
  if (groups.length === 1) {
    for (const prop of groups[0]) always.add(prop);
    return `export interface ${name} {\n${renderProperties(props, always, `${indent}  `).join('\n')}\n${indent}}`;
  }

  if (groups.length === 0) {
    return `export interface ${name} {\n${base.join('\n')}\n${indent}}`;
  }

  // Two or more alternatives: at least one group must be fully satisfied.
  // Each alternative is rendered on its own so the constraint survives.
  const alternatives = groups.map(
    (group) =>
      `${indent}  | {\n${renderProperties(props, group, `${indent}    `).join('\n')}\n${indent}  }`
  );
  return (
    `export type ${name} = {\n${base.join('\n')}\n${indent}} & (\n` +
    `${alternatives.join('\n')}\n${indent});`
  );
}

/** `SMS Sync` -> `SmsSyncMessage` */
function interfaceNameFor(title) {
  return String(title).replace(/\s+/g, '') + 'Message';
}

let tsOutput = `// GENERATED FILE - DO NOT EDIT
// Generated from packages/protocol/schema.json by scripts/generate_types.js
// Run \`npm run generate\` to refresh.

`;

// Shared value objects, referenced by `$ref` from the message branches.
const defs = schema.definitions || {};
for (const [defName, defObj] of Object.entries(defs)) {
  if (defObj.type !== 'object') continue;
  tsOutput += `${renderObject(defName, defObj)}\n\n`;
}

// Map keyed by generated name so a title can never produce two union members.
const unionMembers = new Map();

for (const [index, msg] of (schema.oneOf || []).entries()) {
  if (msg.type !== 'object') continue;

  const title = msg.title;
  if (typeof title !== 'string' || title.length === 0) {
    throw new Error(
      `schema.json: oneOf branch #${index} is missing a "title". ` +
        'Every branch needs a unique title; it becomes the TypeScript ' +
        'interface name and the tag of the WebSocketMessage union.'
    );
  }

  const interfaceName = interfaceNameFor(title);

  // Fail loudly on collision *before* emitting anything: a duplicate title
  // would otherwise silently add a second, identical union member and make
  // the schema's own oneOf ambiguity visible in the generated types.
  if (unionMembers.has(interfaceName)) {
    throw new Error(
      `schema.json: duplicate oneOf title ${JSON.stringify(title)} at ` +
        `branches #${unionMembers.get(interfaceName)} and #${index}; both ` +
        `map to TypeScript interface ${interfaceName}. Rename one of the ` +
        'branches so every oneOf branch generates exactly one type.'
    );
  }
  unionMembers.set(interfaceName, index);

  tsOutput += `${renderObject(interfaceName, msg)}\n\n`;
}

const members = [...unionMembers.keys()];
if (members.length === 0) {
  throw new Error('schema.json: oneOf contains no object branches; nothing to generate.');
}

tsOutput += `export type WebSocketMessage =\n  | ${members.join('\n  | ')};\n\n`;
tsOutput += `export type MessageHandler = (data: WebSocketMessage) => void;\n`;

fs.writeFileSync(TS_OUT_PATH, tsOutput);
console.log(
  `TypeScript definitions generated: ${members.length} message types, ` +
    `${Object.keys(defs).length} definitions -> ${path.relative(process.cwd(), TS_OUT_PATH)}`
);
