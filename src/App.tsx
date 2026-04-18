import { useEffect } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from './store/useStore';
import { TitleBar } from './components/TitleBar';
import { PinDisplay } from './components/PinModal';
import { TransferPanel } from './components/TransferProgress';
import { UpdateBanner } from './components/UpdateBanner';
import { Home } from './pages/Home';
import { SettingsPage } from './pages/Settings';
import { PairingRequest, FileOffer, TransferInfo } from './types';

export default function App() {
  const {
    currentPage,
    setPage,
    setPeers,
    setStatus,
    setSettings,
    pairingRequest,
    setPairingRequest,
    setTransfers,
    setFileOffer,
  } = useStore();

  const refresh = async () => {
    try {
      const [devices, stat, sett] = await Promise.all([
        invoke<any[]>('get_devices'),
        invoke<any>('get_status'),
        invoke<any>('get_settings'),
      ]);
      setPeers(devices);
      setStatus(stat);
      setSettings(sett);
    } catch (e) {
      console.error('refresh error', e);
    }
  };

  useEffect(() => {
    refresh();

    const unlisten = Promise.all([
      listen('peers-updated', refresh),
      listen('connected', refresh),
      listen('disconnected', refresh),
      listen('focus-acquired', refresh),
      listen('focus-released', refresh),
      listen<PairingRequest>('pairing-request', (e) => setPairingRequest(e.payload)),
      listen('pin-rejected', () => {
        setPairingRequest(null);
        alert('Incorrect PIN or session expired. Please try again.');
      }),

      // File transfer events
      listen<TransferInfo[]>('transfer-update', (e) => setTransfers(e.payload)),
      listen<FileOffer>('file-offer', (e) => setFileOffer(e.payload)),
      listen('transfer-complete', (e: any) => {
        const { name } = e.payload ?? {};
        if (name) {
          // Briefly show completion — store already updated via transfer-update
        }
      }),

      // File drop — send to connected peer if any
      listen('tauri://drag-drop', async (e: any) => {
        const paths: string[] = e.payload?.paths ?? [];
        if (!paths.length) return;
        const store = useStore.getState();
        const connectedPeer = store.status?.connected_peer;
        if (!connectedPeer) return;
        try {
          await invoke('send_files', { peerId: connectedPeer, paths });
        } catch (err) {
          console.error('send_files error', err);
        }
      }),
    ]);

    const interval = setInterval(refresh, 3000);

    return () => {
      clearInterval(interval);
      unlisten.then((fns) => fns.forEach((fn) => fn()));
    };
  }, []);

  return (
    <div
      className="w-full h-screen flex flex-col overflow-hidden"
      style={{
        background: 'linear-gradient(160deg, #0f0c29 0%, #1a0d4a 45%, #0d1535 100%)',
        borderRadius: '18px',
        border: '1px solid rgba(99,102,241,0.18)',
        boxShadow: '0 32px 80px rgba(0,0,0,0.7), 0 0 0 1px rgba(255,255,255,0.04) inset, 0 1px 0 rgba(167,139,250,0.12) inset',
      }}
    >
      <TitleBar onSettings={() => setPage(currentPage === 'settings' ? 'home' : 'settings')} />

      <UpdateBanner />
      <TransferPanel />

      <AnimatePresence mode="wait">
        {currentPage === 'home' && (
          <motion.div
            key="home"
            initial={{ opacity: 0, x: -10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }}
            transition={{ duration: 0.15 }}
            className="flex flex-col flex-1 overflow-hidden"
          >
            <Home />
          </motion.div>
        )}
        {currentPage === 'settings' && (
          <motion.div
            key="settings"
            initial={{ opacity: 0, x: 10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -10 }}
            transition={{ duration: 0.15 }}
            className="flex flex-col flex-1 overflow-hidden"
          >
            <SettingsPage />
          </motion.div>
        )}
      </AnimatePresence>

      <AnimatePresence>
        {pairingRequest && (
          <PinDisplay
            pin={pairingRequest.pin}
            peerName={pairingRequest.peer_name}
            onClose={() => setPairingRequest(null)}
          />
        )}
      </AnimatePresence>
    </div>
  );
}
