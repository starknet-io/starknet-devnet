import { useState, useMemo, useRef, useCallback, useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Box, Loader2 } from 'lucide-react';
import { getBlockWithTxHashes, blockNumber as rpcBlockNumber } from '@/lib/rpc-client';
import { devnetGetStatus } from '@/lib/rpc-client';
import { formatTimestamp } from '@/lib/utils';
import SearchBar from '@/components/SearchBar';

const CHUNK_SIZE = 20;
const ORIGIN_BLOCKS_TO_SHOW = 5;

function BlockRow({ bn, isOrigin }: { bn: number; isOrigin: boolean }) {
  const { data: block, isLoading } = useQuery({
    queryKey: ['block', bn],
    queryFn: () => getBlockWithTxHashes(bn),
    staleTime: 5000,
  });

  if (isLoading) {
    return (
      <tr className="border-b border-starknet-border/50">
        <td className="px-4 py-3 font-mono text-gray-500 text-sm">#{bn}</td>
        <td className="px-4 py-3"><Loader2 className="animate-spin text-gray-600" size={12} /></td>
        <td className="px-4 py-3"><span className="text-gray-600 text-xs">loading</span></td>
        <td className="px-4 py-3" /><td className="px-4 py-3" /><td className="px-4 py-3" />
      </tr>
    );
  }

  if (!block) {
    return (
      <tr className="border-b border-starknet-border/50">
        <td className="px-4 py-3 font-mono text-gray-600 text-sm">#{bn}</td>
        <td className="px-4 py-3 text-red-400 text-xs" colSpan={5}>Failed to load</td>
      </tr>
    );
  }

  return (
    <tr className={`transition-colors ${isOrigin ? 'bg-starknet-gold/10 hover:bg-starknet-gold/15' : 'hover:bg-white/[0.045]'}`}>
      <td className="px-4 py-3">
        <Link to={`/blocks/${block.block_number}`} className="flex items-center gap-2 text-starknet-purple hover:underline">
          <Box size={14} />
          <span className="font-mono text-sm">{block.block_number}</span>
        </Link>
      </td>
      <td className="px-4 py-3">
        <Link to={`/blocks/${block.block_number}`} className="hash-link">
          {block.block_hash}
        </Link>
      </td>
      <td className="px-4 py-3">
        {isOrigin ? (
          <span className="pill text-starknet-gold">Origin</span>
        ) : (
          <span className="pill text-starknet-mint">Devnet</span>
        )}
      </td>
      <td className="px-4 py-3"><StatusBadge status={block.status} /></td>
      <td className="px-4 py-3 text-right text-gray-400 text-xs">{formatTimestamp(block.timestamp)}</td>
      <td className="px-4 py-3 text-right">
        <span className="px-2 py-0.5 rounded bg-starknet-purple/20 text-starknet-purple text-xs">{block.transactions.length}</span>
      </td>
    </tr>
  );
}

export default function BlocksPage() {
  const [loadedCount, setLoadedCount] = useState(CHUNK_SIZE);
  const sentinelRef = useRef<HTMLDivElement>(null);

  const { data: status } = useQuery({
    queryKey: ['status'],
    queryFn: devnetGetStatus,
    refetchInterval: 4000,
  });

  const { data: latestBlockNum } = useQuery({
    queryKey: ['blockNumber'],
    queryFn: rpcBlockNumber,
    refetchInterval: 4000,
  });

  const totalDevnetBlocks = status?.block_count ?? 0;
  const isForked = status?.is_forked ?? false;
  const forkBlock = status?.fork_config?.block ?? 0;

  const allBlockNums = useMemo(() => {
    if (latestBlockNum == null || totalDevnetBlocks === 0) return [];
    const latest = latestBlockNum;
    const devnetEnd = isForked ? forkBlock + 1 : latest - totalDevnetBlocks + 1;
    const nums: { n: number; isOrigin: boolean }[] = [];
    for (let n = latest; n >= Math.max(0, devnetEnd); n--) {
      nums.push({ n, isOrigin: false });
    }
    if (isForked && forkBlock >= 0) {
      for (let n = forkBlock; n >= Math.max(0, forkBlock - ORIGIN_BLOCKS_TO_SHOW + 1); n--) {
        nums.push({ n, isOrigin: true });
      }
    }
    return nums;
  }, [totalDevnetBlocks, latestBlockNum, isForked, forkBlock]);

  const visibleNums = useMemo(
    () => allBlockNums.slice(0, loadedCount),
    [allBlockNums, loadedCount],
  );

  const handleIntersect = useCallback(
    (entries: IntersectionObserverEntry[]) => {
      if (entries[0].isIntersecting && loadedCount < allBlockNums.length) {
        setLoadedCount((c) => Math.min(c + CHUNK_SIZE, allBlockNums.length));
      }
    },
    [loadedCount, allBlockNums.length],
  );

  useEffect(() => {
    const el = sentinelRef.current;
    if (!el) return;
    const obs = new IntersectionObserver(handleIntersect, { rootMargin: '200px' });
    obs.observe(el);
    return () => obs.disconnect();
  }, [handleIntersect]);

  useEffect(() => {
    setLoadedCount(CHUNK_SIZE);
  }, [latestBlockNum]);

  const hasMore = loadedCount < allBlockNums.length;

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Blocks</h1>
          <p className="page-subtitle">{allBlockNums.length.toLocaleString()} blocks</p>
        </div>
        <SearchBar />
      </div>

      {allBlockNums.length > 0 && (
        <div className="card overflow-hidden p-0">
          <div className="overflow-x-auto">
            <table className="data-table">
              <thead>
                <tr>
                  <th>Block</th>
                  <th>Hash</th>
                  <th>Source</th>
                  <th>Status</th>
                  <th className="text-right">Age</th>
                  <th className="text-right">TXs</th>
                </tr>
              </thead>
              <tbody>
                {visibleNums.map(({ n, isOrigin }) => (
                  <BlockRow key={n} bn={n} isOrigin={isOrigin} />
                ))}
              </tbody>
            </table>
          </div>
          <div ref={sentinelRef} className="flex items-center justify-center py-6">
            {hasMore ? (
              <Loader2 className="animate-spin text-starknet-purple" size={20} />
            ) : (
              <span className="text-gray-500 text-xs">All blocks loaded</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const color =
    status === 'ACCEPTED_ON_L2' ? 'bg-green-500/20 text-green-400 border-green-500/30'
    : status === 'ACCEPTED_ON_L1' ? 'bg-blue-500/20 text-blue-400 border-blue-500/30'
    : status === 'PRE_CONFIRMED' ? 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30'
    : status === 'ABORTED' ? 'bg-red-500/20 text-red-400 border-red-500/30'
    : 'bg-gray-500/20 text-gray-400 border-gray-500/30';
  return (
    <span className={`px-2 py-0.5 rounded text-xs font-medium border ${color}`}>
      {status.replace(/_/g, ' ')}
    </span>
  );
}
