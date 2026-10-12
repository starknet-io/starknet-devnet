import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/rpc-client.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source.replace('import.meta.env.DEV', 'isDev'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

function client({ isDev = false, storedUrl = null, fetch } = {}) {
  const exports = {};
  runInNewContext(compiled, {
    exports, isDev, fetch,
    require: () => ({ useSyncExternalStore: () => {} }),
    window: { location: { origin: 'http://127.0.0.1:60600' } },
    localStorage: { getItem: () => storedUrl, setItem: () => {} },
  });
  return exports;
}

test('embedded UI defaults to its server, with development and saved endpoint overrides', () => {
  assert.equal(client().getRpcUrl(), 'http://127.0.0.1:60600/rpc');
  assert.equal(client({ isDev: true }).getRpcUrl(), 'http://127.0.0.1:5050/rpc');
  assert.equal(client({ storedUrl: 'http://other/rpc' }).getRpcUrl(), 'http://other/rpc');
});

test('requests use the currently selected endpoint and preserve snapshot response types', async () => {
  let result = '0xa';
  const api = client({ fetch: async (url, options) => {
    assert.equal(url, 'http://other/rpc');
    const body = JSON.parse(options.body);
    assert.equal(body.jsonrpc, '2.0');
    assert.equal(body.method, result === '0xa' ? 'devnet_snapshot' : 'devnet_revert');
    assert.deepEqual(body.params, result === '0xa' ? {} : { snapshot_id: '0xa' });
    return { ok: true, json: async () => ({ result }) };
  } });
  api.setRpcUrl('http://other/rpc');
  assert.equal(await api.callRpc('devnet_snapshot', {}), '0xa');
  result = false;
  assert.equal(await api.callRpc('devnet_revert', { snapshot_id: '0xa' }), false);
});

test('RPC validation errors retain field context from the server', async () => {
  const api = client({ fetch: async () => ({ ok: true, json: async () => ({
    error: { code: -32602, message: 'Invalid params', data: 'classes[0].class_hash does not match definition' },
  }) }) });
  await assert.rejects(api.callRpc('devnet_applyStateDiff', {}), /classes\[0\]\.class_hash/);
});
