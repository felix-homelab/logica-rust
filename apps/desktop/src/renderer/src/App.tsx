// The window's routes: the workbench, the settings window (#settings) and the component gallery
// (#gallery, development builds only).
import { useEffect, useState } from 'react';
import { TooltipProvider } from '@logica/ui';
import { Gallery } from './Gallery';
import { Settings } from './Settings';
import { StoreProvider } from './store';
import { Workbench } from './workbench/Workbench';

export function App() {
  const [route, setRoute] = useState(window.location.hash.slice(1));
  useEffect(() => {
    const f = () => setRoute(window.location.hash.slice(1));
    window.addEventListener('hashchange', f);
    return () => window.removeEventListener('hashchange', f);
  }, []);
  const [packaged, setPackaged] = useState(true);
  useEffect(() => window.logica.onEnv((e) => setPackaged(e.packaged)), []);
  // development builds only (doc 08)
  const gallery = route === 'gallery' && (import.meta.env.DEV || !packaged);
  return (
    <StoreProvider>
      <TooltipProvider delayDuration={400}>{gallery ? <Gallery /> : route === 'settings' ? <Settings /> : <Workbench />}</TooltipProvider>
    </StoreProvider>
  );
}
