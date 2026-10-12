import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../src/lib/event-decoder.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

// Exercise the decoder's public API with an isolated module cache and a mock RPC transport.
function decoder(callRpc, getRpcUrl = () => 'http://devnet/rpc') {
  const module = { exports: {} };
  runInNewContext(compiled, {
    exports: module.exports,
    require: (name) => name === '@/lib/rpc-client' ? { callRpc, getRpcUrl } : require(name),
  });
  return module.exports;
}

function contractClass(name) {
  return {
    abi: [{ type: 'function', name, inputs: [], outputs: [], state_mutability: 'view' }],
    entry_points_by_type: { EXTERNAL: [], CONSTRUCTOR: [], L1_HANDLER: [] },
  };
}

test('mutable tags and block numbers observe class replacement', async () => {
  for (const blockId of [undefined, 'latest', 'pre_confirmed', { block_number: 1 }]) {
    let name = 'before';
    let calls = 0;
    const api = decoder(async () => { calls++; return contractClass(name); });
    const before = await api.decodeFunctionSelector('0x1', api.selectorFromName(name), [], blockId);
    assert.equal(before.name, 'before');
    name = 'after';
    const after = await api.decodeFunctionSelector('0x1', api.selectorFromName(name), [], blockId);
    assert.equal(after.name, 'after');
    assert.equal(calls, 2);
  }
});

test('missing classes can be decoded after they become available', async () => {
  for (const classHash of [undefined, '0x456']) {
    for (const failure of ['error', 'null']) {
      let available = false;
      const api = decoder(async () => {
        if (available) return contractClass('declared');
        if (failure === 'null') return null;
        throw new Error('Class not found');
      });
      const blockId = { block_hash: '0x123' };
      const decode = () => api.decodeFunctionSelector('0x1', api.selectorFromName('declared'), [], blockId, [], classHash);
      assert.equal((await decode()).source, 'raw');
      available = true;
      assert.equal((await decode()).name, 'declared');
    }
  }
});

test('immutable block lookups stay cached and are scoped to the RPC endpoint', async () => {
  for (const classHash of [undefined, '0x456']) {
    let url = 'http://first/rpc';
    let name = 'first';
    let calls = 0;
    const api = decoder(async () => { calls++; return contractClass(name); }, () => url);
    const blockId = { block_hash: '0x123' };
    const first = () => api.decodeFunctionSelector('0x1', api.selectorFromName('first'), [], blockId, [], classHash);
    assert.equal((await first()).name, 'first');
    name = 'second';
    assert.equal((await first()).name, 'first');
    assert.equal(calls, 1);
    url = 'http://second/rpc';
    const second = await api.decodeFunctionSelector('0x1', api.selectorFromName('second'), [], blockId, [], classHash);
    assert.equal(second.name, 'second');
    assert.equal(calls, 2);
  }
});

test('historical lookups that fall back to latest are not cached', async () => {
  for (const classHash of [undefined, '0x456']) {
    let name = 'before';
    const api = decoder(async (method, params) => {
      assert.equal(method, classHash ? 'starknet_getClass' : 'starknet_getClassAt');
      if (params.block_id !== 'latest') throw new Error('Historical state unavailable');
      return contractClass(name);
    });
    const blockId = { block_hash: '0x123' };
    const before = await api.decodeFunctionSelector('0x1', api.selectorFromName(name), [], blockId, [], classHash);
    assert.equal(before.name, 'before');
    name = 'after';
    const after = await api.decodeFunctionSelector('0x1', api.selectorFromName(name), [], blockId, [], classHash);
    assert.equal(after.name, 'after');
  }
});

test('concurrent immutable lookups share the in-flight request', async () => {
  let complete;
  let calls = 0;
  const api = decoder(() => {
    calls++;
    return new Promise((resolve) => { complete = resolve; });
  });
  const decode = () => api.decodeFunctionSelector('0x1', api.selectorFromName('read'), [], { block_hash: '0x123' });
  const first = decode();
  const second = decode();
  assert.equal(calls, 1);
  complete(contractClass('read'));
  for (const result of await Promise.all([first, second])) assert.equal(result.name, 'read');
});
