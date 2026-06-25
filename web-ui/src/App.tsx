import { Routes, Route, NavLink } from 'react-router-dom';
import {
  LayoutDashboard,
  Blocks,
  Settings,
  Users,
  Gamepad2,
  Activity,
} from 'lucide-react';
import Dashboard from './pages/Dashboard';
import BlocksPage from './pages/BlocksPage';
import BlockDetail from './pages/BlockDetail';
import TxDetail from './pages/TxDetail';
import ConfigPage from './pages/ConfigPage';
import AccountsPage from './pages/AccountsPage';
import ControlPanel from './pages/ControlPanel';
import { DevnetProvider } from './lib/DevnetContext';

const navGroups = [
  {
    label: 'Overview',
    items: [{ to: '/', icon: LayoutDashboard, label: 'Dashboard' }],
  },
  {
    label: 'Explorer',
    items: [{ to: '/blocks', icon: Blocks, label: 'Blocks' }],
  },
  {
    label: 'Devnet',
    items: [
      { to: '/config', icon: Settings, label: 'Config' },
      { to: '/accounts', icon: Users, label: 'Accounts' },
      { to: '/control', icon: Gamepad2, label: 'Control Panel' },
    ],
  },
];

function NavItem({
  to,
  icon: Icon,
  label,
  compact = false,
}: {
  to: string;
  icon: React.ElementType;
  label: string;
  compact?: boolean;
}) {
  return (
    <NavLink
      to={to}
      end
      className={({ isActive }) =>
        `flex items-center gap-3 rounded-lg text-sm font-medium transition-colors ${
          compact ? 'px-3 py-2 shrink-0' : 'px-4 py-2.5'
        } ${
          isActive
            ? 'bg-starknet-accent/[0.15] text-white shadow-glow border border-starknet-accent/25'
            : 'text-slate-400 hover:text-white hover:bg-white/[0.06] border border-transparent'
        }`
      }
    >
      <Icon size={18} />
      <span>{label}</span>
    </NavLink>
  );
}

export default function App() {
  return (
    <DevnetProvider>
      <div className="app-shell">
        <aside className="app-sidebar">
          <div className="px-5 py-5 border-b border-white/10">
            <div className="flex items-center gap-3">
              <div className="grid h-11 w-11 place-items-center rounded-lg bg-starknet-accent text-[#170b10] shadow-glow">
                <Activity size={24} />
              </div>
              <div>
                <h1 className="text-lg font-semibold text-white">Devnet Explorer</h1>
                <p className="text-xs text-slate-500 mt-0.5">starknet-devnet-rs</p>
              </div>
            </div>
          </div>

          <nav className="flex-1 px-3 py-4 space-y-5 overflow-y-auto">
            {navGroups.map((group) => (
              <div key={group.label}>
                <p className="px-4 text-[11px] font-semibold text-slate-500 uppercase tracking-[0.18em] mb-2">
                  {group.label}
                </p>
                <div className="space-y-1">
                  {group.items.map((item) => (
                    <NavItem key={item.to} {...item} />
                  ))}
                </div>
              </div>
            ))}
          </nav>

          <div className="px-4 py-4 border-t border-white/10">
            <div className="rounded-lg border border-white/10 bg-white/[0.04] px-3 py-3 text-xs text-slate-400">
              <span className="block font-mono text-starknet-mint">v0.9.0</span>
              <span className="block mt-1">Local Starknet workspace</span>
            </div>
          </div>
        </aside>

        <div className="app-mobile-nav">
          <div className="px-4 py-3 flex items-center gap-3 border-b border-white/10">
            <div className="grid h-9 w-9 place-items-center rounded-lg bg-starknet-accent text-[#170b10]">
              <Activity size={20} />
            </div>
            <div>
              <h1 className="text-sm font-semibold text-white">Devnet Explorer</h1>
              <p className="text-[11px] text-slate-500">starknet-devnet-rs</p>
            </div>
          </div>
          <nav className="flex gap-2 overflow-x-auto px-3 py-2">
            {navGroups.flatMap((group) => group.items).map((item) => (
              <NavItem key={item.to} {...item} compact />
            ))}
          </nav>
        </div>

        <main className="app-main">
          <Routes>
            <Route path="/" element={<Dashboard />} />
            <Route path="/blocks" element={<BlocksPage />} />
            <Route path="/blocks/:blockNumber" element={<BlockDetail />} />
            <Route path="/tx/:txHash" element={<TxDetail />} />
            <Route path="/config" element={<ConfigPage />} />
            <Route path="/accounts" element={<AccountsPage />} />
            <Route path="/control" element={<ControlPanel />} />
          </Routes>
        </main>
      </div>
    </DevnetProvider>
  );
}
