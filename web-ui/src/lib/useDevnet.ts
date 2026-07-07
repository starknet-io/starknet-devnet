// React hook for consuming DevnetContext. Kept in its own file so the
// context provider file (DevnetContext.tsx) only exports the component,
// keeping React Fast Refresh working cleanly.

import { useContext } from 'react';
import { DevnetContext, type DevnetContextType } from './DevnetContext';

export function useDevnet(): DevnetContextType {
  return useContext(DevnetContext);
}
