import { useMemo, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import {
  Activity,
  AlertTriangle,
  ArrowLeft,
  CheckCircle,
  ChevronDown,
  ChevronRight,
  Code2,
  Globe,
  Loader2,
  XCircle,
} from 'lucide-react';
import {
  getTransactionByHash,
  getTransactionReceipt,
  getTransactionStatus,
  getTransactionTrace,
} from '@/lib/rpc-client';
import { CopyableHash, formatFee } from '@/components/CopyableHash';
import {
  blockIdKey,
  decodeReceiptEvents,
  decodeTrace,
  normalizeFelt,
  type DecodedEvent,
  type DecodedField,
  type DecodedInvocation,
  type DecodedTrace,
  type DecodedTraceSection,
  type RpcBlockId,
} from '@/lib/event-decoder';

export default function TxDetail() {
  const { txHash } = useParams<{ txHash: string }>();
  const navigate = useNavigate();

  const { data: tx, isLoading: txLoading } = useQuery({
    queryKey: ['tx', txHash],
    queryFn: () => getTransactionByHash(txHash!),
    enabled: !!txHash,
  });

  const { data: receipt } = useQuery({
    queryKey: ['receipt', txHash],
    queryFn: () => getTransactionReceipt(txHash!),
    enabled: !!txHash,
  });

  const { data: status } = useQuery({
    queryKey: ['txstatus', txHash],
    queryFn: () => getTransactionStatus(txHash!),
    enabled: !!txHash,
  });

  const { data: trace, isLoading: traceLoading } = useQuery({
    queryKey: ['trace', txHash],
    queryFn: () => getTransactionTrace(txHash!),
    enabled: !!txHash,
    retry: false,
  });

  const blockId = useMemo(() => getReceiptBlockId(receipt), [receipt]);
  const blockKey = blockIdKey(blockId);
  const receiptEvents = Array.isArray((receipt as any)?.events) ? ((receipt as any).events as any[]) : [];
  const eventFingerprint = JSON.stringify(
    receiptEvents.map((event) => ({
      from_address: event.from_address,
      keys: event.keys ?? [],
      data: event.data ?? [],
    })),
  );

  const { data: decodedEvents, isLoading: eventsDecoding } = useQuery({
    queryKey: ['decoded-events', txHash, blockKey, eventFingerprint],
    queryFn: () => decodeReceiptEvents(receiptEvents, blockId),
    enabled: receiptEvents.length > 0,
  });

  const { data: decodedTrace, isLoading: traceDecoding } = useQuery({
    queryKey: ['decoded-trace', txHash, blockKey],
    queryFn: () => decodeTrace(trace, blockId),
    enabled: !!trace,
  });

  if (txLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <Loader2 className="animate-spin text-starknet-purple" size={32} />
      </div>
    );
  }

  if (!tx) return <div className="p-8 text-red-400">Transaction not found</div>;

  const raw = tx as any;
  const rawReceipt = receipt as any;
  const txType = raw.type || 'INVOKE';

  return (
    <div className="page">
      <button
        onClick={() => navigate(-1)}
        className="flex items-center gap-1 text-gray-400 hover:text-gray-200 text-sm transition-colors"
      >
        <ArrowLeft size={14} />
        Back
      </button>

      <div className="page-header">
        <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-3 mb-3">
          <TxTypeBadge type={txType} />
          {status && (
            <ExecutionBadge
              finality={status.finality_status}
              execution={status.execution_status}
            />
          )}
          {rawReceipt?.finality_status && <StatusBadge label={rawReceipt.finality_status} />}
        </div>
        <CopyableHash value={txHash ?? ''} />
        </div>
      </div>

      <Section title="Overview">
        <Grid>
          <F label="Type" value={txType} />
          {raw.version != null && <F label="Version" value={String(raw.version)} mono />}
          {raw.nonce != null && <F label="Nonce" value={String(raw.nonce)} mono />}
          {raw.sender_address && <F label="Sender" value={raw.sender_address} hash />}
          {raw.contract_address && <F label="Contract" value={raw.contract_address} hash />}
          {rawReceipt?.contract_address && <F label="Deployed Contract" value={rawReceipt.contract_address} hash />}
          {raw.entry_point_selector && <F label="Entry Point Selector" value={raw.entry_point_selector} hash />}
          {raw.class_hash && <F label="Class Hash" value={raw.class_hash} hash />}
          {raw.compiled_class_hash && <F label="Compiled Class Hash" value={raw.compiled_class_hash} hash />}
          {raw.contract_address_salt && <F label="Salt" value={raw.contract_address_salt} hash />}
        </Grid>
      </Section>

      {rawReceipt && (
        <Section title="Block Info">
          <Grid>
            {rawReceipt.block_number != null && (
              <F label="Block" value={`#${rawReceipt.block_number}`} link={`/blocks/${rawReceipt.block_number}`} />
            )}
            {rawReceipt.block_hash && <F label="Block Hash" value={rawReceipt.block_hash} hash />}
            {rawReceipt.transaction_hash && <F label="Receipt Hash" value={rawReceipt.transaction_hash} hash />}
          </Grid>
        </Section>
      )}

      {rawReceipt && (
        <Section title="Fees">
          <Grid>
            {rawReceipt.actual_fee && (
              <>
                <F
                  label="Fee Paid"
                  value={`${formatFee(String(rawReceipt.actual_fee.amount))} ${
                    rawReceipt.actual_fee.unit === 'WEI' ? 'ETH' : 'STRK'
                  }`}
                />
                <F label="Fee (hex)" value={String(rawReceipt.actual_fee.amount)} mono />
              </>
            )}
            {rawReceipt.execution_resources && (
              <>
                <F label="L1 Gas" value={bigFmt(rawReceipt.execution_resources.l1_gas)} />
                <F label="L1 Data Gas" value={bigFmt(rawReceipt.execution_resources.l1_data_gas)} />
                <F label="L2 Gas" value={bigFmt(rawReceipt.execution_resources.l2_gas)} />
              </>
            )}
          </Grid>
        </Section>
      )}

      <DecodedCallsSection
        decodedTrace={decodedTrace}
        loading={traceLoading || traceDecoding}
        hasRawTrace={!!trace}
      />

      {receiptEvents.length > 0 && (
        <Section title={`Events (${receiptEvents.length})`} loading={eventsDecoding}>
          <div className="space-y-3">
            {receiptEvents.map((event, index) => (
              <EventRow
                key={`${event.from_address ?? 'event'}-${index}`}
                event={event}
                decoded={decodedEvents?.[index]}
                loading={eventsDecoding}
                index={index}
              />
            ))}
          </div>
        </Section>
      )}

      {Array.isArray(raw.calldata) && raw.calldata.length > 0 && (
        <Section title={`Transaction Calldata (${raw.calldata.length})`} collapsible>
          <Arr items={raw.calldata} />
        </Section>
      )}

      {Array.isArray(raw.constructor_calldata) && raw.constructor_calldata.length > 0 && (
        <Section title={`Constructor Calldata (${raw.constructor_calldata.length})`} collapsible>
          <Arr items={raw.constructor_calldata} />
        </Section>
      )}

      {Array.isArray(raw.signature) && raw.signature.length > 0 && (
        <Section title={`Signature (${raw.signature.length})`} collapsible>
          <Arr items={raw.signature} />
        </Section>
      )}

      {Array.isArray(rawReceipt?.messages_sent) && rawReceipt.messages_sent.length > 0 && (
        <Section title={`L2 to L1 Messages (${rawReceipt.messages_sent.length})`}>
          <div className="space-y-2">
            {rawReceipt.messages_sent.map((message: any, index: number) => (
              <div key={index} className="rounded-lg bg-starknet-surface border border-starknet-border p-3">
                <Grid>
                  {message.to_address && <F label="To" value={message.to_address} hash />}
                  {message.from_address && <F label="From" value={message.from_address} hash />}
                </Grid>
                {Array.isArray(message.payload) && message.payload.length > 0 && (
                  <div className="mt-3">
                    <Arr items={message.payload} dense />
                  </div>
                )}
              </div>
            ))}
          </div>
        </Section>
      )}

      <Section title="Raw Trace" loading={traceLoading} collapsible>
        {trace ? (
          <pre className="text-xs font-mono text-gray-300 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {JSON.stringify(trace, null, 2)}
          </pre>
        ) : traceLoading ? null : (
          <p className="text-gray-500 text-sm">Not available</p>
        )}
      </Section>
    </div>
  );
}

function DecodedCallsSection({
  decodedTrace,
  loading,
  hasRawTrace,
}: {
  decodedTrace?: DecodedTrace;
  loading: boolean;
  hasRawTrace: boolean;
}) {
  return (
    <Section title="Decoded Calls" loading={loading}>
      {decodedTrace?.sections.length ? (
        <div className="space-y-4">
          {decodedTrace.sections.map((section) => (
            <TraceSectionView key={section.name} section={section} />
          ))}
        </div>
      ) : loading ? null : (
        <div className="flex items-center gap-2 text-sm text-gray-500">
          {hasRawTrace ? <AlertTriangle size={14} /> : <Code2 size={14} />}
          <span>{hasRawTrace ? 'No invocations in trace' : 'Trace not available'}</span>
        </div>
      )}
    </Section>
  );
}

function TraceSectionView({ section }: { section: DecodedTraceSection }) {
  if (section.revertReason) {
    return (
      <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3">
        <div className="flex items-center gap-2 text-red-300 text-sm font-medium">
          <AlertTriangle size={14} />
          {section.name} reverted
        </div>
        <p className="mt-2 text-xs text-red-200 break-all font-mono">{section.revertReason}</p>
      </div>
    );
  }

  if (!section.invocation) return null;
  return <InvocationCard invocation={section.invocation} label={section.name} depth={0} />;
}

function InvocationCard({
  invocation,
  label,
  depth,
}: {
  invocation: DecodedInvocation;
  label?: string;
  depth: number;
}) {
  const decoded = invocation.decoded;
  const title = decoded.name ?? shortenSelector(invocation.selector);
  const [open, setOpen] = useState(() => depth < 2 && !isAccountWrapperCall(decoded.name, label));

  return (
    <div
      className="rounded-lg bg-starknet-surface border border-starknet-border overflow-hidden"
      style={{ marginLeft: depth ? Math.min(depth * 18, 72) : 0 }}
    >
      <button
        onClick={() => setOpen((value) => !value)}
        className="w-full px-3 py-3 flex items-center justify-between gap-3 text-left hover:bg-starknet-border/40 transition-colors"
      >
        <div className="flex items-center gap-2 min-w-0">
          {open ? <ChevronDown size={14} className="text-gray-500 shrink-0" /> : <ChevronRight size={14} className="text-gray-500 shrink-0" />}
          <Activity size={14} className={invocation.isReverted ? 'text-red-400 shrink-0' : 'text-starknet-purple shrink-0'} />
          {label && <span className="text-[11px] uppercase tracking-wide text-gray-500 shrink-0">{label}</span>}
          <span className="font-mono text-sm text-gray-100 break-all">{title}</span>
          <DecodeBadge source={decoded.source} />
        </div>
        <div className="flex items-center gap-3 shrink-0">
          {invocation.calls.length > 0 && <Count label="calls" value={invocation.calls.length} />}
          {invocation.events.length > 0 && <Count label="events" value={invocation.events.length} />}
        </div>
      </button>

      {open && (
        <div className="px-3 pb-3 space-y-4">
          <Grid>
            <F label="Contract" value={invocation.contractAddress} hash />
            {invocation.callerAddress && <F label="Caller" value={invocation.callerAddress} hash />}
            {invocation.classHash && <F label="Class Hash" value={invocation.classHash} hash />}
            <F label="Selector" value={invocation.selector} hash />
            {invocation.entryPointType && <F label="Entry Point" value={invocation.entryPointType} />}
            {invocation.callType && <F label="Call Type" value={invocation.callType} />}
          </Grid>

          {decoded.inputs.length > 0 && (
            <FieldGroup title="Inputs" fields={decoded.inputs} />
          )}

          {decoded.unmatchedCalldata.length > 0 && (
            <RawGroup title={`Unmatched Calldata (${decoded.unmatchedCalldata.length})`} items={decoded.unmatchedCalldata} />
          )}

          {decoded.outputs.length > 0 && (
            <FieldGroup title="Result" fields={decoded.outputs} />
          )}

          {decoded.unmatchedResult.length > 0 && (
            <RawGroup title={`Raw Result (${decoded.unmatchedResult.length})`} items={decoded.unmatchedResult} />
          )}

          {invocation.events.length > 0 && (
            <div>
              <h3 className="text-xs font-semibold text-gray-400 mb-2">Direct Events</h3>
              <div className="space-y-2">
                {invocation.events.map((event, index) => (
                  <EventRow
                    key={`${event.fromAddress}-${index}`}
                    event={{ from_address: event.fromAddress, keys: event.rawKeys, data: event.rawData }}
                    decoded={event}
                    index={index}
                  />
                ))}
              </div>
            </div>
          )}

          {invocation.calls.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-semibold text-gray-400">Nested Calls</h3>
              {invocation.calls.map((call, index) => (
                <InvocationCard
                  key={`${call.contractAddress}-${call.selector}-${index}`}
                  invocation={call}
                  depth={depth + 1}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function EventRow({
  event,
  decoded,
  loading,
  index,
}: {
  event: any;
  decoded?: DecodedEvent;
  loading?: boolean;
  index: number;
}) {
  const name = decoded?.name ?? normalizeFelt(event.keys?.[0] ?? 'Unknown event');
  const source = decoded?.source ?? 'raw';

  return (
    <div className="p-4 rounded-lg bg-starknet-surface border border-starknet-border">
      <div className="flex items-center justify-between gap-3 mb-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-xs text-gray-500 shrink-0">#{index}</span>
          <span
            className={`px-2 py-0.5 rounded text-xs font-medium border break-all ${
              source === 'abi'
                ? 'bg-blue-500/20 text-blue-300 border-blue-500/30'
                : 'bg-gray-500/20 text-gray-400 border-gray-500/30'
            }`}
          >
            {name}
          </span>
          {source === 'abi' ? (
            <Globe size={11} className="text-blue-400 shrink-0" aria-label="Decoded from on-chain ABI" />
          ) : loading ? (
            <Loader2 size={11} className="animate-spin text-gray-600 shrink-0" />
          ) : null}
        </div>
        <CopyableHash value={event.from_address ?? decoded?.fromAddress ?? ''} short={8} className="text-xs shrink-0" />
      </div>

      {decoded?.selectorNames.length ? (
        <div className="mb-2 flex flex-wrap gap-1.5">
          {decoded.selectorNames.map((selectorName, selectorIndex) => (
            <span key={`${selectorName}-${selectorIndex}`} className="text-[10px] px-1.5 py-0.5 rounded bg-starknet-card text-gray-400 border border-starknet-border">
              {selectorName}
            </span>
          ))}
        </div>
      ) : null}

      {decoded?.fields.length ? (
        <DecodedFields fields={decoded.fields} compact />
      ) : null}

      <details className="mt-2">
        <summary className="text-[10px] text-gray-600 cursor-pointer hover:text-gray-400">Raw event</summary>
        <div className="mt-1 space-y-0.5">
          <RawKeyValues label="key" items={event.keys ?? decoded?.rawKeys ?? []} />
          <RawKeyValues label="data" items={event.data ?? decoded?.rawData ?? []} />
        </div>
      </details>
    </div>
  );
}

function FieldGroup({ title, fields }: { title: string; fields: DecodedField[] }) {
  return (
    <div>
      <h3 className="text-xs font-semibold text-gray-400 mb-2">{title}</h3>
      <DecodedFields fields={fields} />
    </div>
  );
}

function DecodedFields({ fields, compact = false }: { fields: DecodedField[]; compact?: boolean }) {
  return (
    <div className={compact ? 'space-y-1 text-xs' : 'space-y-1.5 text-xs'}>
      {fields.map((field, index) => (
        <DecodedFieldRow key={`${field.name}-${index}`} field={field} compact={compact} />
      ))}
    </div>
  );
}

function DecodedFieldRow({ field, compact }: { field: DecodedField; compact: boolean }) {
  const hasChildren = Array.isArray(field.children) && field.children.length > 0;
  const isArray = isArrayDecodedField(field);
  const [open, setOpen] = useState(() => hasChildren && !isArray);

  return (
    <div className="rounded border border-starknet-border/70 bg-starknet-card/40 px-2 py-1.5">
      <div className="grid grid-cols-[minmax(96px,180px)_1fr] gap-3 items-start">
        <div className="min-w-0">
          <div className="flex items-center gap-1 min-w-0 text-gray-400">
            {hasChildren && (
              <button
                onClick={() => setOpen((value) => !value)}
                className="text-gray-500 hover:text-gray-300 shrink-0"
                aria-label={open ? 'Collapse decoded field' : 'Expand decoded field'}
              >
                {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
              </button>
            )}
            <span className="break-words">{field.name}</span>
          </div>
          {field.type && <div className="text-[10px] text-gray-600 break-all">{lastSegment(field.type)}</div>}
        </div>
        <ValueCell value={field.value} raw={field.raw} />
      </div>

      {hasChildren && open && (
        <div className={`mt-1.5 border-l border-starknet-border pl-2 ${compact ? 'space-y-1' : 'space-y-1.5'}`}>
          {field.children!.map((child, index) => (
            <DecodedFieldRow key={`${child.name}-${index}`} field={child} compact />
          ))}
        </div>
      )}
    </div>
  );
}

function isArrayDecodedField(field: DecodedField): boolean {
  const type = field.type ?? '';
  return type.includes('Array::<') || type.includes('Span::<') || type.includes('Array<') || type.includes('Span<');
}

function isAccountWrapperCall(name?: string, label?: string): boolean {
  return name === '__validate__' || label === 'Validate';
}

function ValueCell({ value, raw }: { value: string; raw: string[] }) {
  const valueIsRawFelt = raw.length === 1 && normalizeFelt(raw[0]) === normalizeFelt(value);

  return (
    <div className="min-w-0 text-right">
      {looksLikeFelt(value) ? (
        <CopyableHash value={value} short={12} className="justify-end max-w-full" />
      ) : (
        <span className="text-gray-200 break-all">{value}</span>
      )}

      {!valueIsRawFelt && raw.length > 0 && (
        <details className="mt-1">
          <summary className="text-[10px] text-gray-600 cursor-pointer hover:text-gray-400">raw</summary>
          <div className="mt-1 space-y-0.5">
            {raw.map((item, index) => (
              <div key={index} className="flex items-center justify-end gap-2 text-[10px] text-gray-500">
                <span>[{index}]</span>
                <CopyableHash value={item} short={8} className="text-[10px]" />
              </div>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

function RawGroup({ title, items }: { title: string; items: string[] }) {
  return (
    <details>
      <summary className="text-xs text-gray-500 cursor-pointer hover:text-gray-300">{title}</summary>
      <div className="mt-2">
        <Arr items={items} dense />
      </div>
    </details>
  );
}

function Section({
  title,
  children,
  loading,
  collapsible,
}: {
  title: string;
  children: ReactNode;
  loading?: boolean;
  collapsible?: boolean;
}) {
  const [open, setOpen] = useState(!collapsible);

  return (
    <div className="card">
      <div className="flex items-center justify-between gap-3 mb-4">
        <h2 className="font-semibold text-sm flex items-center gap-2">
          {collapsible && (
            <button
              onClick={() => setOpen((value) => !value)}
              className="text-gray-400 hover:text-gray-200"
              aria-label={open ? 'Collapse section' : 'Expand section'}
            >
              {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </button>
          )}
          {title}
        </h2>
        {loading && <Loader2 className="animate-spin text-gray-500" size={14} />}
      </div>
      {open && children}
    </div>
  );
}

function Grid({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-x-8 gap-y-3 text-sm">{children}</dl>;
}

function F({
  label,
  value,
  mono,
  hash,
  link,
}: {
  label: string;
  value?: string;
  mono?: boolean;
  hash?: boolean;
  link?: string;
}) {
  if (value == null || value === '') return null;

  const display = link ? (
    <Link to={link} className="text-starknet-purple hover:underline text-left break-all cursor-pointer font-mono text-sm">
      {value}
    </Link>
  ) : hash ? (
    <CopyableHash value={value} short={12} />
  ) : mono ? (
    <span className="font-mono text-sm text-gray-300 break-all">{value}</span>
  ) : (
    <span className="text-sm text-gray-200 break-all">{value}</span>
  );

  return (
    <div>
      <dt className="text-gray-500 text-xs mb-0.5">{label}</dt>
      <dd className="text-gray-200 flex items-center gap-2 min-w-0">{display}</dd>
    </div>
  );
}

function Arr({ items, dense = false }: { items: string[]; dense?: boolean }) {
  return (
    <div className={`${dense ? 'space-y-0' : 'space-y-0.5'} max-h-80 overflow-y-auto`}>
      {items.map((item, index) => (
        <div
          key={`${item}-${index}`}
          className={`font-mono text-xs flex items-start gap-3 px-2 rounded hover:bg-starknet-surface/50 ${
            dense ? 'py-0.5' : 'py-1'
          }`}
        >
          <span className="text-gray-500 w-10 shrink-0 select-none">[{index}]</span>
          <CopyableHash value={String(item)} />
        </div>
      ))}
    </div>
  );
}

function RawKeyValues({ label, items }: { label: string; items: string[] }) {
  return (
    <>
      {items.map((item, index) => (
        <div key={`${label}-${index}`} className="font-mono text-[11px] pl-2 text-gray-500 break-all flex items-center gap-2">
          <span className="shrink-0">{label}[{index}]</span>
          <CopyableHash value={String(item)} className="text-[11px]" />
        </div>
      ))}
    </>
  );
}

function DecodeBadge({ source }: { source: 'abi' | 'entrypoint' | 'raw' }) {
  const className =
    source === 'abi'
      ? 'bg-blue-500/20 text-blue-300 border-blue-500/30'
      : source === 'entrypoint'
        ? 'bg-green-500/20 text-green-300 border-green-500/30'
        : 'bg-gray-500/20 text-gray-400 border-gray-500/30';
  const label = source === 'abi' ? 'ABI' : source === 'entrypoint' ? 'ENTRYPOINT' : 'RAW';

  return <span className={`px-1.5 py-0.5 rounded text-[10px] font-medium border ${className}`}>{label}</span>;
}

function Count({ label, value }: { label: string; value: number }) {
  return (
    <span className="text-[10px] uppercase tracking-wide text-gray-500">
      {value} {label}
    </span>
  );
}

function TxTypeBadge({ type }: { type: string }) {
  const colors: Record<string, string> = {
    INVOKE: 'bg-purple-500/20 text-purple-400 border-purple-500/30',
    DEPLOY: 'bg-green-500/20 text-green-400 border-green-500/30',
    DECLARE: 'bg-blue-500/20 text-blue-400 border-blue-500/30',
    DEPLOY_ACCOUNT: 'bg-teal-500/20 text-teal-400 border-teal-500/30',
    L1_HANDLER: 'bg-orange-500/20 text-orange-400 border-orange-500/30',
  };

  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border uppercase ${colors[type] || 'bg-gray-500/20 text-gray-400 border-gray-500/30'}`}>
      {type}
    </span>
  );
}

function ExecutionBadge({ execution }: { finality: string; execution: string }) {
  const ok = execution === 'SUCCEEDED';
  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border flex items-center gap-1 ${ok ? 'bg-green-500/20 text-green-400 border-green-500/30' : 'bg-red-500/20 text-red-400 border-red-500/30'}`}>
      {ok ? <CheckCircle size={10} /> : <XCircle size={10} />}
      {execution}
    </span>
  );
}

function StatusBadge({ label }: { label: string }) {
  const color =
    label === 'ACCEPTED_ON_L2'
      ? 'bg-green-500/20 text-green-400 border-green-500/30'
      : label === 'ACCEPTED_ON_L1'
        ? 'bg-blue-500/20 text-blue-400 border-blue-500/30'
        : label === 'PRE_CONFIRMED'
          ? 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30'
          : 'bg-gray-500/20 text-gray-400 border-gray-500/30';

  return <span className={`px-2 py-0.5 rounded text-xs font-medium border ${color}`}>{label.replace(/_/g, ' ')}</span>;
}

function getReceiptBlockId(receipt: unknown): RpcBlockId | undefined {
  const raw = receipt as any;
  if (raw?.block_hash) return { block_hash: raw.block_hash };
  if (raw?.block_number != null) return { block_number: Number(raw.block_number) };
  return undefined;
}

function looksLikeFelt(value: string): boolean {
  return /^0x[0-9a-fA-F]+$/.test(value);
}

function shortenSelector(selector: string): string {
  if (!selector) return 'Unknown selector';
  return `${selector.slice(0, 10)}...${selector.slice(-8)}`;
}

function lastSegment(value: string): string {
  const cairoParts = value.split('::');
  return cairoParts[cairoParts.length - 1] ?? value;
}

function bigFmt(value: unknown): string {
  if (value == null) return '0';

  try {
    return BigInt(String(value)).toLocaleString();
  } catch {
    return String(value);
  }
}
