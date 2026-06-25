import { keccak256 } from 'js-sha3';
import { callRpc } from '@/lib/rpc-client';

export type RpcBlockId = 'latest' | 'pre_confirmed' | { block_hash: string } | { block_number: number };

export interface RawEvent {
  from_address: string;
  class_hash?: string;
  keys?: string[];
  data?: string[];
}

export interface DecodedField {
  name: string;
  type?: string;
  value: string;
  raw: string[];
  source: 'calldata' | 'result' | 'key' | 'data';
  children?: DecodedField[];
}

export interface DecodedEvent {
  fromAddress: string;
  name: string;
  fullName?: string;
  source: 'abi' | 'raw';
  selectorPath: string[];
  selectorNames: string[];
  fields: DecodedField[];
  unmatchedKeys: string[];
  unmatchedData: string[];
  rawKeys: string[];
  rawData: string[];
  error?: string;
}

export interface DecodedFunction {
  name?: string;
  fullName?: string;
  selector: string;
  source: 'abi' | 'entrypoint' | 'raw';
  inputs: DecodedField[];
  outputs: DecodedField[];
  unmatchedCalldata: string[];
  unmatchedResult: string[];
  rawCalldata: string[];
  rawResult: string[];
}

export interface DecodedInvocation {
  contractAddress: string;
  selector: string;
  calldata: string[];
  callerAddress?: string;
  classHash?: string;
  entryPointType?: string;
  callType?: string;
  result: string[];
  calls: DecodedInvocation[];
  events: DecodedEvent[];
  messages: unknown[];
  executionResources?: unknown;
  isReverted?: boolean;
  decoded: DecodedFunction;
}

export interface DecodedTraceSection {
  name: string;
  invocation?: DecodedInvocation;
  revertReason?: string;
}

export interface DecodedTrace {
  type?: string;
  sections: DecodedTraceSection[];
  raw: unknown;
}

interface AbiParam {
  name?: string;
  type?: string;
  kind?: string;
  [key: string]: unknown;
}

interface AbiEntry {
  type?: string;
  name?: string;
  kind?: string;
  selector?: string;
  inputs?: AbiParam[];
  outputs?: AbiParam[];
  members?: AbiParam[];
  variants?: AbiParam[];
  items?: AbiEntry[];
  keys?: AbiParam[];
  data?: AbiParam[];
  [key: string]: unknown;
}

interface ContractClassInfo {
  abi: AbiEntry[];
  entryPoints: Set<string>;
}

interface AbiFunctionEntry {
  name: string;
  fullName: string;
  selectors: string[];
  inputs: AbiParam[];
  outputs: AbiParam[];
}

interface EventPattern {
  name: string;
  fullName?: string;
  selectors: string[];
  selectorNames: string[];
  members: AbiParam[];
}

interface AbiContext {
  typeByName: Map<string, AbiEntry>;
  functionEntries: AbiFunctionEntry[];
  eventPatterns: EventPattern[];
}

interface DecodedValue {
  value: string;
  raw: string[];
  children?: DecodedField[];
  next: number;
}

interface FieldDecodeResult {
  fields: DecodedField[];
  next: number;
}

interface RawInvocation {
  contract_address?: string;
  entry_point_selector?: string;
  calldata?: string[];
  caller_address?: string;
  class_hash?: string;
  entry_point_type?: string;
  call_type?: string;
  result?: string[];
  calls?: RawInvocation[];
  events?: Array<{ keys?: string[]; data?: string[]; order?: number }>;
  messages?: unknown[];
  execution_resources?: unknown;
  is_reverted?: boolean;
}

const classAtCache = new Map<string, Promise<ContractClassInfo | null>>();
const classHashCache = new Map<string, Promise<ContractClassInfo | null>>();
const abiContextCache = new WeakMap<AbiEntry[], AbiContext>();

export function selectorFromName(name: string): string {
  const bytes = keccak256.array(name);
  bytes[0] &= 0x03;
  return normalizeFelt(`0x${bytes.map((byte) => byte.toString(16).padStart(2, '0')).join('')}`);
}

export function normalizeFelt(value: unknown): string {
  if (typeof value !== 'string') return String(value ?? '');
  const trimmed = value.trim();
  if (!trimmed) return trimmed;

  try {
    return `0x${BigInt(trimmed).toString(16)}`;
  } catch {
    return trimmed.toLowerCase();
  }
}

export function blockIdKey(blockId?: RpcBlockId): string {
  if (!blockId) return 'latest';
  if (typeof blockId === 'string') return blockId;
  if ('block_hash' in blockId) return `hash:${normalizeFelt(blockId.block_hash)}`;
  return `number:${blockId.block_number}`;
}

export async function decodeReceiptEvents(
  events: RawEvent[],
  blockId?: RpcBlockId,
): Promise<DecodedEvent[]> {
  return Promise.all(events.map((event) => decodeEvent(event, blockId)));
}

export async function decodeEvent(event: RawEvent, blockId?: RpcBlockId): Promise<DecodedEvent> {
  const fromAddress = String(event.from_address ?? '');
  const classHash = event.class_hash ? String(event.class_hash) : undefined;
  const rawKeys = asStringArray(event.keys);
  const rawData = asStringArray(event.data);

  try {
    const classInfo = await fetchContractClass({ address: fromAddress, classHash, blockId });
    const context = classInfo?.abi ? getAbiContext(classInfo.abi) : null;
    const pattern = context ? findEventPattern(context.eventPatterns, rawKeys) : null;

    if (pattern && context) {
      const keysAfterSelectors = rawKeys.slice(pattern.selectors.length);
      const { fields, keyIndex, dataIndex } = decodeEventMembers(
        pattern.members,
        keysAfterSelectors,
        rawData,
        context,
      );

      return {
        fromAddress,
        name: pattern.name,
        fullName: pattern.fullName,
        source: 'abi',
        selectorPath: pattern.selectors,
        selectorNames: pattern.selectorNames,
        fields,
        unmatchedKeys: keysAfterSelectors.slice(keyIndex),
        unmatchedData: rawData.slice(dataIndex),
        rawKeys,
        rawData,
      };
    }

    return rawEvent(fromAddress, rawKeys, rawData);
  } catch (error) {
    return {
      ...rawEvent(fromAddress, rawKeys, rawData),
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

export async function decodeFunctionSelector(
  contractAddress: string,
  selector: string,
  calldata: string[],
  blockId?: RpcBlockId,
  result: string[] = [],
  classHash?: string,
): Promise<DecodedFunction> {
  const normalizedSelector = normalizeFelt(selector);
  const rawCalldata = asStringArray(calldata);
  const rawResult = asStringArray(result);
  const classInfo = await fetchContractClass({ address: contractAddress, classHash, blockId });
  const context = classInfo?.abi ? getAbiContext(classInfo.abi) : null;
  const abiFunction = context?.functionEntries.find((entry) =>
    entry.selectors.some((candidate) => sameFelt(candidate, normalizedSelector)),
  );

  if (abiFunction && context) {
    const decodedInputs = decodeFields(abiFunction.inputs, rawCalldata, 'calldata', context);
    const decodedOutputs = decodeFields(abiFunction.outputs, rawResult, 'result', context);

    return {
      name: abiFunction.name,
      fullName: abiFunction.fullName,
      selector: normalizedSelector,
      source: 'abi',
      inputs: decodedInputs.fields,
      outputs: decodedOutputs.fields,
      unmatchedCalldata: rawCalldata.slice(decodedInputs.next),
      unmatchedResult: rawResult.slice(decodedOutputs.next),
      rawCalldata,
      rawResult,
    };
  }

  return {
    selector: normalizedSelector,
    source: classInfo?.entryPoints.has(normalizedSelector) ? 'entrypoint' : 'raw',
    inputs: [],
    outputs: [],
    unmatchedCalldata: rawCalldata,
    unmatchedResult: rawResult,
    rawCalldata,
    rawResult,
  };
}

export async function decodeTrace(trace: unknown, blockId?: RpcBlockId): Promise<DecodedTrace> {
  const root = trace as Record<string, unknown>;
  const sections: DecodedTraceSection[] = [];

  await addInvocationSection(sections, 'Validate', root.validate_invocation, blockId);
  await addInvocationSection(sections, 'Execute', root.execute_invocation, blockId);
  await addInvocationSection(sections, 'Constructor', root.constructor_invocation, blockId);
  await addInvocationSection(sections, 'Function', root.function_invocation, blockId);
  await addInvocationSection(sections, 'Fee Transfer', root.fee_transfer_invocation, blockId);

  return {
    type: typeof root.type === 'string' ? root.type : undefined,
    sections,
    raw: trace,
  };
}

async function addInvocationSection(
  sections: DecodedTraceSection[],
  name: string,
  value: unknown,
  blockId?: RpcBlockId,
): Promise<void> {
  if (!value) return;

  if (isReversion(value)) {
    sections.push({ name, revertReason: String(value.revert_reason) });
    return;
  }

  if (isRawInvocation(value)) {
    sections.push({
      name,
      invocation: await decodeInvocation(value, blockId),
    });
  }
}

async function decodeInvocation(invocation: RawInvocation, blockId?: RpcBlockId): Promise<DecodedInvocation> {
  const contractAddress = String(invocation.contract_address ?? '');
  const selector = String(invocation.entry_point_selector ?? '');
  const calldata = asStringArray(invocation.calldata);
  const result = asStringArray(invocation.result);
  const rawCalls = Array.isArray(invocation.calls) ? invocation.calls : [];
  const rawEvents = Array.isArray(invocation.events) ? invocation.events : [];

  const [decoded, calls, events] = await Promise.all([
    decodeFunctionSelector(contractAddress, selector, calldata, blockId, result, invocation.class_hash),
    Promise.all(rawCalls.filter(isRawInvocation).map((call) => decodeInvocation(call, blockId))),
    Promise.all(
      rawEvents.map((event) =>
        decodeEvent(
          {
            from_address: contractAddress,
            class_hash: invocation.class_hash,
            keys: asStringArray(event.keys),
            data: asStringArray(event.data),
          },
          blockId,
        ),
      ),
    ),
  ]);

  return {
    contractAddress,
    selector: normalizeFelt(selector),
    calldata,
    callerAddress: invocation.caller_address,
    classHash: invocation.class_hash,
    entryPointType: invocation.entry_point_type,
    callType: invocation.call_type,
    result,
    calls,
    events,
    messages: Array.isArray(invocation.messages) ? invocation.messages : [],
    executionResources: invocation.execution_resources,
    isReverted: invocation.is_reverted,
    decoded,
  };
}

async function fetchContractClass({
  address,
  classHash,
  blockId,
}: {
  address?: string;
  classHash?: string;
  blockId?: RpcBlockId;
}): Promise<ContractClassInfo | null> {
  if (classHash) {
    const byHash = await fetchClassByHash(classHash, blockId);
    if (byHash) return byHash;
  }

  return address ? fetchClassAt(address, blockId) : null;
}

async function fetchClassByHash(classHash: string, blockId?: RpcBlockId): Promise<ContractClassInfo | null> {
  if (!classHash) return null;

  const cacheKey = `${blockIdKey(blockId)}:${normalizeFelt(classHash)}`;
  const cached = classHashCache.get(cacheKey);
  if (cached) return cached;

  const promise = fetchClassByHashUncached(classHash, blockId);
  classHashCache.set(cacheKey, promise);
  return promise;
}

async function fetchClassByHashUncached(
  classHash: string,
  blockId?: RpcBlockId,
): Promise<ContractClassInfo | null> {
  const requestedBlock = blockId ?? 'latest';

  try {
    return parseContractClass(
      await callRpc<Record<string, unknown>>('starknet_getClass', {
        block_id: requestedBlock,
        class_hash: classHash,
      }),
    );
  } catch {
    if (blockId && blockId !== 'latest') {
      try {
        return parseContractClass(
          await callRpc<Record<string, unknown>>('starknet_getClass', {
            block_id: 'latest',
            class_hash: classHash,
          }),
        );
      } catch {
        return null;
      }
    }

    return null;
  }
}

async function fetchClassAt(address: string, blockId?: RpcBlockId): Promise<ContractClassInfo | null> {
  if (!address) return null;

  const cacheKey = `${blockIdKey(blockId)}:${normalizeFelt(address)}`;
  const cached = classAtCache.get(cacheKey);
  if (cached) return cached;

  const promise = fetchClassAtUncached(address, blockId);
  classAtCache.set(cacheKey, promise);
  return promise;
}

async function fetchClassAtUncached(
  address: string,
  blockId?: RpcBlockId,
): Promise<ContractClassInfo | null> {
  const requestedBlock = blockId ?? 'latest';

  try {
    return parseContractClass(
      await callRpc<Record<string, unknown>>('starknet_getClassAt', {
        block_id: requestedBlock,
        contract_address: address,
      }),
    );
  } catch {
    if (blockId && blockId !== 'latest') {
      try {
        return parseContractClass(
          await callRpc<Record<string, unknown>>('starknet_getClassAt', {
            block_id: 'latest',
            contract_address: address,
          }),
        );
      } catch {
        return null;
      }
    }

    return null;
  }
}

function parseContractClass(contractClass: Record<string, unknown> | null): ContractClassInfo | null {
  if (!contractClass) return null;

  const abi = parseAbi(contractClass.abi);
  const entryPoints = new Set<string>();
  const byType = contractClass.entry_points_by_type;

  if (byType && typeof byType === 'object') {
    for (const entries of Object.values(byType as Record<string, unknown>)) {
      if (!Array.isArray(entries)) continue;
      for (const entry of entries) {
        if (entry && typeof entry === 'object' && 'selector' in entry) {
          const selector = (entry as { selector?: unknown }).selector;
          if (selector != null) entryPoints.add(normalizeFelt(selector));
        }
      }
    }
  }

  return { abi, entryPoints };
}

function parseAbi(rawAbi: unknown): AbiEntry[] {
  if (Array.isArray(rawAbi)) return rawAbi as AbiEntry[];
  if (typeof rawAbi !== 'string' || rawAbi.trim() === '') return [];

  try {
    const parsed = JSON.parse(rawAbi);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function getAbiContext(abi: AbiEntry[]): AbiContext {
  const cached = abiContextCache.get(abi);
  if (cached) return cached;

  const typeByName = new Map<string, AbiEntry>();
  for (const entry of abi) {
    if (!entry.name) continue;
    if (entry.type === 'struct' || entry.type === 'enum' || entry.type === 'event') {
      typeByName.set(entry.name, entry);
    }
  }

  const context: AbiContext = {
    typeByName,
    functionEntries: collectFunctionEntries(abi),
    eventPatterns: collectEventPatterns(abi, typeByName),
  };

  abiContextCache.set(abi, context);
  return context;
}

function collectFunctionEntries(abi: AbiEntry[]): AbiFunctionEntry[] {
  const functions: AbiFunctionEntry[] = [];

  for (const entry of abi) {
    if (isCallableAbiEntry(entry)) {
      functions.push(toFunctionEntry(entry, entry.name ?? '<anonymous>'));
    }

    if (entry.type === 'interface' && Array.isArray(entry.items)) {
      for (const item of entry.items) {
        if (isCallableAbiEntry(item)) {
          functions.push(toFunctionEntry(item, `${entry.name ?? '<interface>'}::${item.name ?? '<anonymous>'}`));
        }
      }
    }
  }

  return functions;
}

function toFunctionEntry(entry: AbiEntry, fullName: string): AbiFunctionEntry {
  const name = entry.name ?? '<anonymous>';

  return {
    name,
    fullName,
    selectors: uniqueFelts([entry.selector, selectorFromName(name), selectorFromName(lastSegment(name))]),
    inputs: Array.isArray(entry.inputs) ? entry.inputs : [],
    outputs: Array.isArray(entry.outputs) ? entry.outputs : [],
  };
}

function collectEventPatterns(abi: AbiEntry[], typeByName: Map<string, AbiEntry>): EventPattern[] {
  const patterns: EventPattern[] = [];
  const eventEntries = abi.filter((entry) => entry.type === 'event' && entry.name);

  for (const event of eventEntries) {
    if (Array.isArray(event.keys) || Array.isArray(event.data)) {
      patterns.push({
        name: event.name ?? '<event>',
        fullName: event.name,
        selectors: [selectorFromName(event.name ?? '')],
        selectorNames: [event.name ?? '<event>'],
        members: [
          ...(event.keys ?? []).map((member) => ({ ...member, kind: 'key' })),
          ...(event.data ?? []).map((member) => ({ ...member, kind: 'data' })),
        ],
      });
      continue;
    }

    if (event.kind === 'struct') {
      patterns.push({
        name: lastSegment(event.name ?? '<event>'),
        fullName: event.name,
        selectors: [selectorFromName(lastSegment(event.name ?? ''))],
        selectorNames: [lastSegment(event.name ?? '<event>')],
        members: Array.isArray(event.members) ? event.members : [],
      });
      continue;
    }

    if (event.kind === 'enum') {
      collectEnumEventPatterns(event, typeByName, [], [], patterns);
    }
  }

  const seen = new Set<string>();
  return patterns
    .filter((pattern) => {
      const key = `${pattern.selectors.map(normalizeFelt).join('/')}:${pattern.fullName ?? pattern.name}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.selectors.length - left.selectors.length);
}

function collectEnumEventPatterns(
  event: AbiEntry,
  typeByName: Map<string, AbiEntry>,
  prefixSelectors: string[],
  prefixNames: string[],
  output: EventPattern[],
): void {
  const variants = Array.isArray(event.variants) ? event.variants : [];

  for (const variant of variants) {
    const variantName = variant.name ?? '<variant>';
    const flattened = variant.kind === 'flat';
    const selectors = flattened ? prefixSelectors : [...prefixSelectors, selectorFromName(variantName)];
    const selectorNames = flattened ? prefixNames : [...prefixNames, variantName];
    const child = variant.type ? typeByName.get(variant.type) : undefined;

    if (child?.type === 'event' && child.kind === 'enum') {
      collectEnumEventPatterns(child, typeByName, selectors, selectorNames, output);
      continue;
    }

    if (child?.type === 'event' && child.kind === 'struct') {
      output.push({
        name: variantName,
        fullName: child.name,
        selectors,
        selectorNames,
        members: Array.isArray(child.members) ? child.members : [],
      });
      continue;
    }

    output.push({
      name: variantName,
      fullName: variant.type,
      selectors,
      selectorNames,
      members: variant.type && variant.type !== '()' ? [{ name: 'value', type: variant.type, kind: 'data' }] : [],
    });
  }
}

function findEventPattern(patterns: EventPattern[], keys: string[]): EventPattern | null {
  return (
    patterns.find((pattern) =>
      pattern.selectors.length > 0
      && pattern.selectors.length <= keys.length
      && pattern.selectors.every((selector, index) => sameFelt(selector, keys[index])),
    ) ?? null
  );
}

function decodeEventMembers(
  members: AbiParam[],
  keys: string[],
  data: string[],
  context: AbiContext,
): { fields: DecodedField[]; keyIndex: number; dataIndex: number } {
  const fields: DecodedField[] = [];
  let keyIndex = 0;
  let dataIndex = 0;

  for (const member of members) {
    const source = member.kind === 'key' ? 'key' : 'data';
    const stream = source === 'key' ? keys : data;
    const start = source === 'key' ? keyIndex : dataIndex;
    const decoded = decodeValue(member.type, stream, start, source, context);

    fields.push({
      name: member.name ?? `[${fields.length}]`,
      type: member.type,
      value: decoded.value,
      raw: decoded.raw,
      source,
      children: decoded.children,
    });

    if (source === 'key') keyIndex = decoded.next;
    else dataIndex = decoded.next;
  }

  return { fields, keyIndex, dataIndex };
}

function decodeFields(
  params: AbiParam[],
  values: string[],
  source: 'calldata' | 'result',
  context: AbiContext,
): FieldDecodeResult {
  const fields: DecodedField[] = [];
  let next = 0;

  for (const param of params) {
    const decoded = decodeValue(param.type, values, next, source, context);
    fields.push({
      name: param.name || `[${fields.length}]`,
      type: param.type,
      value: decoded.value,
      raw: decoded.raw,
      source,
      children: decoded.children,
    });
    next = decoded.next;
  }

  return { fields, next };
}

function decodeValue(
  type: string | undefined,
  values: string[],
  index: number,
  source: DecodedField['source'],
  context: AbiContext,
): DecodedValue {
  if (index >= values.length) return { value: '<missing>', raw: [], next: index };

  const normalizedType = normalizeType(type);
  const arrayItemType = arrayInnerType(normalizedType);

  if (arrayItemType) {
    const length = feltToSafeNumber(values[index]);
    if (length == null || length > 128) {
      return {
        value: length == null ? 'array' : `${length.toLocaleString()} items`,
        raw: [values[index]],
        next: index + 1,
      };
    }

    const children: DecodedField[] = [];
    let cursor = index + 1;
    for (let i = 0; i < length; i += 1) {
      const decoded = decodeValue(arrayItemType, values, cursor, source, context);
      children.push({
        name: `[${i}]`,
        type: arrayItemType,
        value: decoded.value,
        raw: decoded.raw,
        source,
        children: decoded.children,
      });
      cursor = decoded.next;
    }

    return {
      value: `${length.toLocaleString()} item${length === 1 ? '' : 's'}`,
      raw: values.slice(index, cursor),
      children,
      next: cursor,
    };
  }

  if (isByteArrayType(normalizedType)) {
    return decodeByteArray(values, index);
  }

  if (isUint256Type(normalizedType)) {
    const low = values[index];
    const high = values[index + 1];
    const raw = high == null ? [low] : [low, high];

    return {
      value: high == null ? low : formatUint256(low, high),
      raw: [],
      next: index + raw.length,
    };
  }

  if (isBoolType(normalizedType)) {
    const felt = normalizeFelt(values[index]);
    return {
      value: felt === '0x0' ? 'false' : felt === '0x1' ? 'true' : values[index],
      raw: [values[index]],
      next: index + 1,
    };
  }

  const typeDef = resolveTypeDef(normalizedType, context);
  if (typeDef?.type === 'struct' || (typeDef?.type === 'event' && typeDef.kind === 'struct')) {
    const members = Array.isArray(typeDef.members) ? typeDef.members : [];
    const children: DecodedField[] = [];
    let cursor = index;

    for (const member of members) {
      const decoded = decodeValue(member.type, values, cursor, source, context);
      children.push({
        name: member.name ?? `[${children.length}]`,
        type: member.type,
        value: decoded.value,
        raw: decoded.raw,
        source,
        children: decoded.children,
      });
      cursor = decoded.next;
    }

    return {
      value: lastSegment(typeDef.name ?? normalizedType),
      raw: values.slice(index, cursor),
      children,
      next: cursor,
    };
  }

  if (typeDef?.type === 'enum' || (typeDef?.type === 'event' && typeDef.kind === 'enum')) {
    const variantIndex = feltToSafeNumber(values[index]);
    const variants = Array.isArray(typeDef.variants) ? typeDef.variants : [];
    const variant = variantIndex == null ? undefined : variants[variantIndex];
    const children: DecodedField[] = [
      {
        name: 'variant',
        type: 'usize',
        value: variant?.name ?? values[index],
        raw: [values[index]],
        source,
      },
    ];
    let cursor = index + 1;

    if (variant?.type && variant.type !== '()') {
      const decoded = decodeValue(variant.type, values, cursor, source, context);
      children.push({
        name: variant.name ?? 'value',
        type: variant.type,
        value: decoded.value,
        raw: decoded.raw,
        source,
        children: decoded.children,
      });
      cursor = decoded.next;
    }

    return {
      value: variant?.name ?? lastSegment(typeDef.name ?? normalizedType),
      raw: values.slice(index, cursor),
      children,
      next: cursor,
    };
  }

  return {
    value: formatPrimitiveValue(normalizedType, values[index]),
    raw: [values[index]],
    next: index + 1,
  };
}

function decodeByteArray(values: string[], index: number): DecodedValue {
  const wordCount = feltToSafeNumber(values[index]);
  if (wordCount == null || wordCount > 128) {
    return { value: 'ByteArray', raw: [values[index]], next: index + 1 };
  }

  const end = Math.min(values.length, index + 1 + wordCount + 2);
  return {
    value: `${wordCount.toLocaleString()} word${wordCount === 1 ? '' : 's'}`,
    raw: values.slice(index, end),
    next: end,
  };
}

function rawEvent(fromAddress: string, rawKeys: string[], rawData: string[]): DecodedEvent {
  return {
    fromAddress,
    name: rawKeys[0] ? normalizeFelt(rawKeys[0]) : 'Unknown event',
    source: 'raw',
    selectorPath: rawKeys[0] ? [normalizeFelt(rawKeys[0])] : [],
    selectorNames: [],
    fields: [],
    unmatchedKeys: rawKeys,
    unmatchedData: rawData,
    rawKeys,
    rawData,
  };
}

function isCallableAbiEntry(entry: AbiEntry): boolean {
  return entry.type === 'function' || entry.type === 'constructor' || entry.type === 'l1_handler';
}

function isRawInvocation(value: unknown): value is RawInvocation {
  return Boolean(value && typeof value === 'object' && 'contract_address' in value && 'entry_point_selector' in value);
}

function isReversion(value: unknown): value is { revert_reason: unknown } {
  return Boolean(value && typeof value === 'object' && 'revert_reason' in value);
}

function sameFelt(left: unknown, right: unknown): boolean {
  return normalizeFelt(left) === normalizeFelt(right);
}

function uniqueFelts(values: unknown[]): string[] {
  const output: string[] = [];
  const seen = new Set<string>();

  for (const value of values) {
    if (value == null || value === '') continue;
    const normalized = normalizeFelt(value);
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    output.push(normalized);
  }

  return output;
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.map((item) => String(item)) : [];
}

function lastSegment(name: string): string {
  const cairoParts = name.split('::');
  const lastCairoPart = cairoParts[cairoParts.length - 1] ?? name;
  const dotParts = lastCairoPart.split('.');
  return dotParts[dotParts.length - 1] ?? lastCairoPart;
}

function normalizeType(type: string | undefined): string {
  if (!type) return 'felt';
  let current = type.trim();

  while (current.startsWith('@')) current = current.slice(1).trim();

  const boxInner = singleGenericInner(current, ['Box']);
  if (boxInner) return normalizeType(boxInner);

  return current;
}

function arrayInnerType(type: string): string | null {
  return singleGenericInner(type, ['Array', 'Span']);
}

function singleGenericInner(type: string, acceptedBaseNames: string[]): string | null {
  const marker = '::<';
  let genericStart = type.indexOf(marker);
  let base = '';

  if (genericStart >= 0 && type.endsWith('>')) {
    base = type.slice(0, genericStart);
    return acceptedBaseNames.includes(lastSegment(base)) ? type.slice(genericStart + marker.length, -1) : null;
  }

  genericStart = type.indexOf('<');
  if (genericStart >= 0 && type.endsWith('>')) {
    base = type.slice(0, genericStart);
    return acceptedBaseNames.includes(lastSegment(base)) ? type.slice(genericStart + 1, -1) : null;
  }

  return null;
}

function resolveTypeDef(type: string, context: AbiContext): AbiEntry | undefined {
  const exact = context.typeByName.get(type);
  if (exact) return exact;

  return [...context.typeByName.entries()].find(([name]) => lastSegment(name) === lastSegment(type))?.[1];
}

function isUint256Type(type: string): boolean {
  return type === 'Uint256' || type === 'u256' || type.endsWith('::u256');
}

function isBoolType(type: string): boolean {
  return type === 'bool' || type.endsWith('::bool');
}

function isByteArrayType(type: string): boolean {
  return type === 'ByteArray' || type.endsWith('::ByteArray');
}

function formatPrimitiveValue(type: string, value: string): string {
  if (/::u(8|16|32|64|128)$/.test(type) || /^u(8|16|32|64|128)$/.test(type)) {
    try {
      return BigInt(value).toLocaleString();
    } catch {
      return value;
    }
  }

  return value;
}

function formatUint256(low: string, high: string): string {
  try {
    return ((BigInt(high) << 128n) + BigInt(low)).toLocaleString();
  } catch {
    return `${low} / ${high}`;
  }
}

function feltToSafeNumber(value: string): number | null {
  try {
    const parsed = BigInt(value);
    return parsed <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(parsed) : null;
  } catch {
    return null;
  }
}
