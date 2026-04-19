import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface TrackpadInfo {
  url: string;
  port: number;
  token: string;
  qr_svg: string;
}

interface Status {
  running: boolean;
  port?: number;
  clients?: number;
}

export const TrackpadCard = () => {
  const [info, setInfo] = useState<TrackpadInfo | null>(null);
  const [clients, setClients] = useState(0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<Status>('get_trackpad_status')
      .then((s) => {
        if (s.running) {
          // Re-fetch full info with QR if already running from a previous session
          invoke<TrackpadInfo>('start_trackpad').then(setInfo).catch(() => {});
          setClients(s.clients ?? 0);
        }
      })
      .catch(() => {});

    const unsubs = Promise.all([
      listen<number>('trackpad-client-count', (e) => setClients(e.payload)),
      listen<{ running: boolean }>('trackpad-status', (e) => {
        if (!e.payload.running) { setInfo(null); setClients(0); }
      }),
    ]);
    return () => { unsubs.then((fns) => fns.forEach((fn) => fn())); };
  }, []);

  const start = async () => {
    setBusy(true); setErr(null);
    try {
      const i = await invoke<TrackpadInfo>('start_trackpad');
      setInfo(i);
    } catch (e: any) {
      setErr(typeof e === 'string' ? e : 'Could not start trackpad');
    } finally { setBusy(false); }
  };

  const stop = async () => {
    setBusy(true);
    try { await invoke('stop_trackpad'); setInfo(null); setClients(0); }
    finally { setBusy(false); }
  };

  const copy = () => {
    if (!info) return;
    navigator.clipboard?.writeText(info.url).catch(() => {});
  };

  return (
    <div
      className="rounded-2xl p-4"
      style={{ background: 'var(--bg-subtle-2)', border: '1px solid var(--border-subtle)' }}
    >
      <div className="flex items-center justify-between mb-3">
        <p className="text-[10px] font-bold uppercase tracking-widest" style={{ color: 'var(--text-muted)' }}>
          Phone Trackpad
        </p>
        {info && (
          <span
            className="text-[10px] font-bold px-2 py-0.5 rounded-full"
            style={{
              background: clients > 0 ? 'rgba(52,211,153,0.15)' : 'var(--bg-subtle)',
              color: clients > 0 ? 'var(--success)' : 'var(--text-muted)',
            }}
          >
            {clients > 0 ? `${clients} connected` : 'Waiting for phone'}
          </span>
        )}
      </div>

      <AnimatePresence mode="wait">
        {!info ? (
          <motion.div key="off" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
            <p className="text-[11px] leading-relaxed mb-3" style={{ color: 'var(--text-muted)' }}>
              Turn your phone into a trackpad for this computer. Scan the QR code from any phone on the same Wi-Fi — no app install needed.
            </p>
            {err && (
              <p className="text-[11px] mb-2" style={{ color: 'var(--danger)' }}>{err}</p>
            )}
            <button
              onClick={start}
              disabled={busy}
              className="w-full py-2.5 rounded-xl text-xs font-bold transition-all active:scale-[0.97] disabled:opacity-50"
              style={{
                background: 'rgba(99,102,241,0.14)',
                border: '1.5px solid rgba(99,102,241,0.3)',
                color: 'var(--accent-primary)',
              }}
            >
              {busy ? 'Starting…' : 'Enable Phone Trackpad'}
            </button>
          </motion.div>
        ) : (
          <motion.div key="on" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
            <div className="flex gap-3 items-stretch">
              {/* QR background stays white across themes so camera apps can
                  reliably read it — the SVG inside is dark. */}
              <div
                className="flex-shrink-0 rounded-xl p-1.5 flex items-center justify-center"
                style={{ background: '#ffffff', border: '1px solid var(--border-subtle)', width: 128, height: 128 }}
                dangerouslySetInnerHTML={{ __html: info.qr_svg }}
              />
              <div className="flex-1 min-w-0 flex flex-col justify-between">
                <div>
                  <p className="text-[10px] uppercase tracking-widest" style={{ color: 'var(--text-muted)' }}>
                    Scan or visit
                  </p>
                  <p
                    className="text-[11px] font-mono mt-1 break-all cursor-pointer"
                    onClick={copy}
                    title="Click to copy"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    {info.url.replace(/^https?:\/\//, '')}
                  </p>
                </div>
                <button
                  onClick={copy}
                  className="mt-2 py-1.5 rounded-lg text-[10px] font-semibold transition-all active:scale-95"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-body)',
                  }}
                >
                  Copy link
                </button>
              </div>
            </div>

            <button
              onClick={stop}
              disabled={busy}
              className="w-full mt-3 py-2 rounded-xl text-[11px] font-semibold transition-all active:scale-[0.97] disabled:opacity-50"
              style={{
                background: 'rgba(248,113,113,0.1)',
                border: '1px solid rgba(248,113,113,0.28)',
                color: 'var(--danger)',
              }}
            >
              Stop trackpad server
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};
