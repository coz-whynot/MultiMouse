import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { PeerInfo } from '../types';
import { useStore } from '../store/useStore';

/* Rough emoji picker for an app-name heuristic. Extend as needed. */
const appEmoji = (name: string): string => {
  const n = name.toLowerCase();
  if (/(chrome|safari|firefox|edge|browser|arc|opera|brave)/.test(n)) return '🌐';
  if (/(code|vim|emacs|sublime|atom|intellij|pycharm|webstorm|xcode|android studio|editor)/.test(n)) return '📝';
  if (/(terminal|iterm|console|powershell|cmd|bash|zsh|hyper|warp)/.test(n)) return '⌨️';
  if (/(slack|discord|teams|zoom|messages|telegram|whatsapp|signal)/.test(n)) return '💬';
  if (/(mail|outlook|gmail|thunderbird)/.test(n)) return '✉️';
  if (/(music|spotify|itunes|apple music)/.test(n)) return '🎵';
  if (/(photos|photoshop|preview|image|lightroom|figma|sketch)/.test(n)) return '🎨';
  if (/(video|vlc|quicktime|netflix|youtube|obs|final cut|premiere)/.test(n)) return '🎬';
  if (/(finder|explorer|files)/.test(n)) return '📁';
  if (/(notes|obsidian|notion|word|pages|docs)/.test(n)) return '📄';
  return '▢';
};

interface Props {
  connectedPeer?: PeerInfo;
  relaying: boolean;
}

/* ── Latency pill (green ≤30ms, yellow 31-80ms, red >80ms, gray if null) ── */
const LatencyPill = ({ ms }: { ms?: number | null }) => {
  let dotColor: string;
  let textColor: string;
  let bg: string;
  let border: string;
  let label: string;

  if (ms == null) {
    dotColor = 'var(--text-faint)';
    textColor = 'var(--text-muted)';
    bg = 'var(--bg-subtle)';
    border = 'var(--border-subtle)';
    label = '—';
  } else if (ms <= 30) {
    dotColor = '#34d399';
    textColor = '#6ee7b7';
    bg = 'rgba(16,185,129,0.12)';
    border = 'rgba(16,185,129,0.25)';
    label = `${Math.round(ms)} ms`;
  } else if (ms <= 80) {
    dotColor = '#fbbf24';
    textColor = '#fcd34d';
    bg = 'rgba(245,158,11,0.12)';
    border = 'rgba(245,158,11,0.28)';
    label = `${Math.round(ms)} ms`;
  } else {
    dotColor = '#f87171';
    textColor = '#fca5a5';
    bg = 'rgba(239,68,68,0.12)';
    border = 'rgba(239,68,68,0.28)';
    label = `${Math.round(ms)} ms`;
  }

  return (
    <span
      className="flex items-center gap-1.5 rounded-full px-2 py-[3px] text-[10px] font-semibold flex-shrink-0"
      style={{ background: bg, border: `1px solid ${border}`, color: textColor }}
      title={ms == null ? 'No latency data yet' : `${Math.round(ms)} ms round-trip`}
    >
      <span
        className="w-1.5 h-1.5 rounded-full flex-shrink-0"
        style={{ background: dotColor }}
      />
      {label}
    </span>
  );
};

export const ConnectionBanner = ({ connectedPeer, relaying }: Props) => {
  // Theme-dependent colors come through CSS custom properties set on the
  // root [data-theme] node, so we no longer need to branch per-theme here.
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const handleDisconnect = () => invoke('disconnect');
  const handleRelease = () => invoke('release_cursor');

  const blurOn = settings?.privacy_blur_on_relay === true;
  const toggleBlur = async () => {
    if (!settings) return;
    const next = { ...settings, privacy_blur_on_relay: !blurOn };
    try {
      await invoke('update_settings', { settings: next });
      setSettings(next);
    } catch (e) {
      console.error('toggle blur failed', e);
    }
  };

  const [activeWindow, setActiveWindow] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = listen<string>('remote-active-window', (e) => {
      const name = typeof e.payload === 'string' ? e.payload.trim() : '';
      setActiveWindow(name.length > 0 ? name : null);
    });
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // Clear the cached active window the moment relay stops so we don't
  // display stale app names when the user grabs control again later.
  useEffect(() => {
    if (!relaying) setActiveWindow(null);
  }, [relaying]);

  const peerName = connectedPeer?.name ?? '';

  return (
    <AnimatePresence>
      {connectedPeer && (
        <motion.div
          initial={{ opacity: 0, y: -8, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: -8, scale: 0.97 }}
          transition={{ type: 'spring', stiffness: 440, damping: 34 }}
          className="mb-4 rounded-2xl overflow-hidden"
          style={{
            background: relaying
              ? 'linear-gradient(135deg, rgba(99,102,241,0.2) 0%, rgba(168,85,247,0.16) 100%)'
              : 'linear-gradient(135deg, rgba(16,185,129,0.14) 0%, rgba(5,150,105,0.10) 100%)',
            border: `1px solid ${relaying ? 'rgba(99,102,241,0.32)' : 'rgba(16,185,129,0.25)'}`,
            boxShadow: relaying
              ? '0 4px 24px rgba(99,102,241,0.12)'
              : '0 4px 20px rgba(16,185,129,0.08)',
          }}
        >
          <div className="flex items-center gap-3 px-5 py-3">
            {/* Status dot with clean pulse */}
            <div className="relative flex-shrink-0 w-3 h-3">
              <motion.div
                className="absolute inset-0 rounded-full"
                animate={{ scale: [1, 2.2, 1], opacity: [0.6, 0, 0.6] }}
                transition={{ repeat: Infinity, duration: 2, ease: 'easeInOut' }}
                style={{ background: relaying ? '#6366f1' : '#10b981' }}
              />
              <div
                className="w-3 h-3 rounded-full"
                style={{ background: relaying ? '#818cf8' : '#34d399' }}
              />
            </div>

            {/* Text */}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <p
                  className="text-sm font-semibold leading-tight truncate flex items-center gap-1.5"
                  style={{ color: 'var(--text-primary)' }}
                >
                  {relaying ? (
                    <AnimatePresence mode="wait" initial={false}>
                      {activeWindow ? (
                        <motion.span
                          key={`active-${activeWindow}`}
                          initial={{ opacity: 0, y: 4 }}
                          animate={{ opacity: 1, y: 0 }}
                          exit={{ opacity: 0, y: -4 }}
                          transition={{ duration: 0.18 }}
                          className="truncate"
                        >
                          <span className="mr-1">{appEmoji(activeWindow)}</span>
                          Controlling {activeWindow} on {peerName}
                        </motion.span>
                      ) : (
                        <motion.span
                          key="controlling-plain"
                          initial={{ opacity: 0, y: 4 }}
                          animate={{ opacity: 1, y: 0 }}
                          exit={{ opacity: 0, y: -4 }}
                          transition={{ duration: 0.18 }}
                          className="truncate"
                        >
                          Controlling {peerName}
                        </motion.span>
                      )}
                    </AnimatePresence>
                  ) : (
                    <span className="truncate">Linked · {peerName}</span>
                  )}
                </p>
                <LatencyPill ms={connectedPeer.ping_ms} />
              </div>
              <p
                className="text-[11px] mt-0.5"
                style={{ color: 'var(--text-muted)' }}
              >
                {relaying
                  ? 'Forwarding input · Press ESC ESC to release'
                  : 'Push cursor to configured edge to take control'}
              </p>
            </div>

            {/* Action buttons — Release is ALWAYS enabled so the user can use the
                tray menu / keyboard hotkey even when the cursor is unreachable */}
            <div className="flex gap-1.5 flex-shrink-0 items-center">
              {/* Privacy blur quick-toggle */}
              <button
                onClick={toggleBlur}
                title={blurOn ? 'Privacy blur ON — click to disable' : 'Privacy blur OFF — click to enable'}
                className="w-7 h-7 rounded-lg flex items-center justify-center transition-all"
                style={{
                  background: blurOn ? 'rgba(99,102,241,0.2)' : 'var(--bg-subtle)',
                  color: blurOn ? 'var(--accent-primary)' : 'var(--text-muted)',
                  border: `1px solid ${blurOn ? 'var(--border-strong)' : 'var(--border-subtle)'}`,
                }}
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  {blurOn ? (
                    <path strokeLinecap="round" strokeLinejoin="round"
                      d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                  ) : (
                    <path strokeLinecap="round" strokeLinejoin="round"
                      d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                  )}
                </svg>
              </button>

              <button
                onClick={handleRelease}
                className="text-xs px-2.5 py-1.5 rounded-lg font-medium transition-all"
                style={{
                  background: relaying ? 'rgba(99,102,241,0.2)' : 'var(--bg-subtle)',
                  color: relaying ? 'var(--accent-primary)' : 'var(--text-body)',
                  border: `1px solid ${relaying ? 'var(--border-strong)' : 'var(--border-subtle)'}`,
                }}
              >
                Release
              </button>
              <button
                onClick={handleDisconnect}
                className="text-xs px-2.5 py-1.5 rounded-lg font-medium transition-all"
                style={{
                  background: 'rgba(239,68,68,0.12)',
                  color: 'var(--danger)',
                  border: '1px solid rgba(239,68,68,0.28)',
                }}
              >
                End
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
