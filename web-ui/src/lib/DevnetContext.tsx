import { createContext, useState, type ReactNode } from 'react';

export interface DevnetContextType {
  connected: boolean;
  setConnected: (v: boolean) => void;
  verified: boolean;
  setVerified: (v: boolean) => void;
}

export const DevnetContext = createContext<DevnetContextType>({
  connected: false,
  setConnected: () => {},
  verified: false,
  setVerified: () => {},
});

export function DevnetProvider({ children }: { children: ReactNode }) {
  const [connected, setConnected] = useState(false);
  const [verified, setVerified] = useState(false);

  return (
    <DevnetContext.Provider value={{ connected, setConnected, verified, setVerified }}>
      {children}
    </DevnetContext.Provider>
  );
}
