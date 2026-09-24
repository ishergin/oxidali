import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const require = createRequire(join(process.cwd(), 'package.json'));
const ts = require('typescript');

const text = readFileSync(0, 'utf8');
const tsx = process.argv[2] !== 'ts';
const file = ts.createSourceFile(tsx ? 'input.tsx' : 'input.ts', text, ts.ScriptTarget.Latest, true, tsx ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
const seen = new Map();
const jsxTexts = [];

function collect(ranges) {
  for (const r of ranges ?? []) seen.set(r.pos, r.end);
}

const emptyContainers = [];

function commentOnlyContainer(node) {
  if (node.kind !== ts.SyntaxKind.JsxExpression || node.expression) return false;
  const inner = text.slice(node.getStart(file) + 1, node.getEnd() - 1);
  return /\/\*/.test(inner) && inner.replace(/\/\*[\s\S]*?\*\//g, '').trim() === '';
}

function visit(node) {
  if (commentOnlyContainer(node)) {
    emptyContainers.push([node.getStart(file), node.getEnd()]);
    return;
  }
  const jsxText = node.kind === ts.SyntaxKind.JsxText || node.kind === ts.SyntaxKind.JsxTextAllWhiteSpaces;
  if (jsxText) jsxTexts.push([node.getFullStart(), node.getEnd()]);
  if (!jsxText) {
    collect(ts.getLeadingCommentRanges(text, node.getFullStart()));
    collect(ts.getTrailingCommentRanges(text, node.getEnd()));
  }
  for (const child of node.getChildren(file)) visit(child);
}

visit(file);
const insideJsxText = ([a]) => jsxTexts.some(([s, e]) => a >= s && a < e);
const insideContainer = ([a]) => emptyContainers.some(([s, e]) => a >= s && a < e);
const ranges = [...seen.entries(), ...emptyContainers]
  .filter((r) => !insideJsxText(r) && (emptyContainers.some(([s]) => s === r[0]) || !insideContainer(r)))
  .sort((a, b) => a[0] - b[0]);
process.stdout.write(JSON.stringify(ranges));
