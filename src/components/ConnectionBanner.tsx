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
    dotColor = 'rgba(255,255,255,0.3)';
    textColor = 'rgba(255,255,255,0.45)';
    bg = 'rgba(255,255,255,0.06)';
    border = 'rgba(255,255,255,0.1)';
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
  const isLight = useStore((s) => s.settings?.theme === 'light');
  const handleDisconnect = () => invoke('disconnect');
  const handleRelease = () => invoke('release_cursor');

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
  const showActive = relaying && !!activeWindow;

  return (
    <AnimatePresence>
      {connectedPeer && (
        <motion.div
          initial={{ opacity: 0, y: -8, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: -8, scale: 0.97 }}
          transition={{ type: 'spring', stiffness: 440, damping: 34 }}
          className="mx-3 mb-3 rounded-2xl overflow-hidden"
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
          <div className="flex items-center gap-3 px-3.5 py-2.5">
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
                  style={{ color: isLight ? '#1e1b4b' : '#ffffff' }}
                >
                  {relaying ? (
                    <AnimatePresence mode="wait" initial={false}>
                      {showActive ? (
                        <motion.span
                          key={`active-${activeWindow}`}
                          initial={{ opacity: 0, y: 4 }}
                          animate={{ opacity: 1, y: 0 }}
                          exit={{ opacity: 0, y: -4 }}
                          transition={{ duration: 0.18 }}
                          className="truncate"
                        >
                          <span className="mr-1">{appEmoji(activeWindow!)}</span>
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
                style={{ color: isLight ? 'rgba(30,27,75,0.55)' : 'rgba(255,255,255,0.4)' }}
              >
                {relaying
                  ? 'Forwarding input · Press ESC ESC to release'
                  : 'Push cursor to configured edge to take control'}
              </p>
            </div>

            {/* Action buttons — Release is ALWAYS enabled so the user can use the
                tray menu / keyboard hotkey even when the cursor is unreachable */}
            <div className="flex gap-1.5 flex-shrink-0">
              <button
                onClick={handleRelease}
                className="text-xs px-2.5 py-1.5 rounded-lg font-medium transition-all"
                style={{
                  background: relaying ? 'rgba(99,102,241,0.18)' : 'rgba(255,255,255,0.07)',
                  color: relaying ? '#c7d2fe' : 'rgba(255,255,255,0.65)',
                  border: `1px solid ${relaying ? 'rgba(99,102,241,0.32)' : 'rgba(255,255,255,0.1)'}`,
                }}
              >
                Release
              </button>
              <button
                onClick={handleDisconnect}
                className="text-xs px-2.5 py-1.5 rounded-lg font-medium transition-all"
                style={{
                  background: 'rgba(239,68,68,0.1)',
                  color: '#f87171',
                  border: '1px solid rgba(239,68,68,0.18)',
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
