import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Search } from 'lucide-react';
import { isHex, isBlockNumber } from '@/lib/utils';

export default function SearchBar() {
  const [query, setQuery] = useState('');
  const navigate = useNavigate();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = query.trim();
    if (!trimmed) return;

    if (isBlockNumber(trimmed)) {
      navigate(`/blocks/${trimmed}`);
    } else if (isHex(trimmed)) {
      navigate(`/tx/${trimmed}`);
    } else {
      // Assume block number
      navigate(`/blocks/${trimmed}`);
    }
    setQuery('');
  };

  return (
    <form onSubmit={handleSubmit} className="relative w-full sm:w-auto">
      <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-500" />
      <input
        type="text"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Block # or Tx Hash..."
        className="input pl-9 w-full sm:w-80 xl:w-[28rem] text-sm"
      />
    </form>
  );
}
