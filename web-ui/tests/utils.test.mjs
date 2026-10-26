import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/utils.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const exports = {};
runInNewContext(compiled, { exports });

test('snapshot IDs accept the server range without losing precision', () => {
  for (const id of ['0x0', '0x1', '0x000A', '0xffffffffffffffff']) assert.equal(exports.isSnapshotId(id), true);
  for (const id of ['', '1', '0x', '0xg', '-0x1', '0x10000000000000000']) assert.equal(exports.isSnapshotId(id), false);
});

test('block and time inputs reject truncated, negative, and unsafe integers', () => {
  assert.equal(exports.parseInt10(' 42 '), 42);
  assert.equal(exports.parseInt10('0'), 0);
  for (const value of ['', '1.5', '1abc', '-1', '1e3', '9007199254740992']) assert.equal(exports.parseInt10(value), undefined);
});

test('configuration displays each block-generation mode including serialized intervals', () => {
  assert.equal(exports.formatBlockGeneration('transaction'), 'On transaction');
  assert.equal(exports.formatBlockGeneration('demand'), 'On demand');
  assert.equal(exports.formatBlockGeneration({ interval: 5 }), 'Every 5s');
});

test('fractional gas prices convert exactly without floating-point rounding or truncation', () => {
  assert.equal(exports.parseGasPrice('1.001'), 1001000000);
  assert.equal(exports.parseGasPrice('0.000000001'), 1);
  assert.equal(exports.parseGasPrice('9007199.254740991'), Number.MAX_SAFE_INTEGER);
  for (const value of ['', '0', '-1', '1foo', '1e3', '0.0000000001', '9007199.254740992']) assert.equal(exports.parseGasPrice(value), undefined);
});

test('dumps must be a dump object or a legacy event array', () => {
  assert.equal(exports.validateDumpText('{"format":"starknet-devnet-dump"}'), undefined);
  assert.equal(exports.validateDumpText('[]'), undefined);
  assert.equal(exports.validateDumpText('  '), 'Choose or paste a dump');
  assert.equal(exports.validateDumpText('{'), 'The dump is not valid JSON');
  assert.equal(exports.validateDumpText('"dump"'), 'A dump is a JSON object or a legacy event array');
});

test('dump summaries count the genesis block and transactions', () => {
  assert.equal(
    exports.summarizeDump({ blocks: { accepted: [{}, {}] }, transactions: [{}] }),
    'Dump downloaded: 3 blocks, 1 transaction',
  );
  assert.equal(exports.summarizeDump({ blocks: { accepted: [] }, transactions: [] }), 'Dump downloaded: 1 block, 0 transactions');
  assert.equal(exports.summarizeDump(null), 'Dump downloaded');
});
