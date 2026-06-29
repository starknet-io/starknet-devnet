import { createContext, useContext, useState, type ReactNode } from 'react';

interface DevnetContextType {
  connected: boolean;
  setConnected: (v: boolean) => void;
  verified: boolean;
  setVerified: (v: boolean) => void;
  blockCount: number;
  setBlockCount: (v: number) => void;
  txCount: number;
  setTxCount: (v: number) => void;
}

const DevnetContext = createContext<DevnetContextType>({
  connected: false,
  setConnected: () => {},
  verified: false,
  setVerified: () => {},
  blockCount: 0,
  setBlockCount: () => {},
  txCount: 0,
  setTxCount: () => {},
});

export function DevnetProvider({ children }: { children: ReactNode }) {
  const [connected, setConnected] = useState(false);
  const [verified, setVerified] = useState(false);
  const [blockCount, setBlockCount] = useState(0);
  const [txCount, setTxCount] = useState(0);

  return (
    <DevnetContext.Provider value={{ connected, setConnected, verified, setVerified, blockCount, setBlockCount, txCount, setTxCount }}>
      {children}
    </DevnetContext.Provider>
  );
}

export function useDevnet() {
  return useContext(DevnetContext);
}
