import { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from './store/useStore';
import { TitleBar } from './components/TitleBar';
import { PinDisplay } from './components/PinModal';
import { TransferPanel } from './components/TransferProgress';
import { UpdateBanner } from './components/UpdateBanner';
import { BottomNav } from './components/BottomNav';
import { Home } from './pages/Home';
import { Layout } from './pages/Layout';
import { SettingsPage } from './pages/Settings';
import { PairingRequest, FileOffer, TransferInfo } from './types';

export default function App() {
  const {
    currentPage,
    setPage,
    setPeers,
    setStatus,
    setSettings,
    status,
    pairingRequest,
    setPairingRequest,
    setTransfers,
    setFileOffer,
  } = useStore();

  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [draggingFiles, setDraggingFiles] = useState(false);

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
        setErrorMsg('Connection rejected or timed out.');
      }),
      listen<{ error: string }>('connection-failed', (e) => {
        setErrorMsg(e.payload?.error ?? 'Connection failed');
      }),

      // File transfer
      listen<TransferInfo[]>('transfer-update', (e) => setTransfers(e.payload)),
      listen<FileOffer>('file-offer', (e) => setFileOffer(e.payload)),
      listen('transfer-complete', () => {}),

      // File drag tracking
      listen('tauri://drag', () => setDraggingFiles(true)),
      listen('tauri://drag-leave', () => setDraggingFiles(false)),
      listen('tauri://drag-cancelled', () => setDraggingFiles(false)),

      // File drop — send to connected peer
      listen('tauri://drag-drop', async (e: any) => {
        setDraggingFiles(false);
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

  const connectedPeer = status?.connected_peer != null;

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
      <TitleBar />

      <UpdateBanner />

      <AnimatePresence>
        {errorMsg && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mt-1"
          >
            <div className="rounded-xl p-2.5 bg-red-500/10 border border-red-500/20 flex items-center gap-2">
              <svg className="w-3.5 h-3.5 text-red-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <p className="text-[10px] text-red-300 flex-1 min-w-0">{errorMsg}</p>
              <button onClick={() => setErrorMsg(null)} className="text-red-400/50 hover:text-red-300 flex-shrink-0">
                <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <TransferPanel />

      {/* File drag overlay */}
      <AnimatePresence>
        {draggingFiles && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="absolute inset-0 z-40 flex items-center justify-center pointer-events-none"
            style={{ borderRadius: '18px', background: 'rgba(99,102,241,0.18)', backdropFilter: 'blur(2px)', border: '2px dashed rgba(99,102,241,0.6)' }}
          >
            <div className="flex flex-col items-center gap-3">
              <motion.div animate={{ y: [0, -8, 0] }} transition={{ repeat: Infinity, duration: 1.2 }}>
                <svg className="w-12 h-12" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
                </svg>
              </motion.div>
              <p className="text-base font-bold text-white/80">
                {connectedPeer ? 'Drop to send file' : 'No device connected'}
              </p>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Page content */}
      <AnimatePresence mode="wait">
        {currentPage === 'home' && (
          <motion.div key="home" initial={{ opacity: 0, x: -10 }} animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }} transition={{ duration: 0.15 }}
            className="flex flex-col flex-1 overflow-hidden">
            <Home />
          </motion.div>
        )}
        {currentPage === 'layout' && (
          <motion.div key="layout" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -10 }} transition={{ duration: 0.15 }}
            className="flex flex-col flex-1 overflow-hidden">
            <Layout />
          </motion.div>
        )}
        {currentPage === 'settings' && (
          <motion.div key="settings" initial={{ opacity: 0, x: 10 }} animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -10 }} transition={{ duration: 0.15 }}
            className="flex flex-col flex-1 overflow-hidden">
            <SettingsPage />
          </motion.div>
        )}
      </AnimatePresence>

      <BottomNav current={currentPage as any} onChange={setPage as any} connectedPeer={connectedPeer} />

      <AnimatePresence>
        {pairingRequest && (
          <PinDisplay
            peerName={pairingRequest.peer_name}
            onClose={() => setPairingRequest(null)}
          />
        )}
      </AnimatePresence>
    </div>
  );
}
