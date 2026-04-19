import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store/useStore';
import { Settings } from '../types';

type Platform = 'macos' | 'windows' | 'linux' | null;

/* ── Animated cursor crossing between two devices ── */
const CursorCrossAnimation = () => (
  <div className="relative w-full h-40 flex items-center justify-between px-4">
    {/* Left device */}
    <div
      className="w-24 h-20 rounded-xl flex items-center justify-center flex-shrink-0 relative"
      style={{
        background: 'linear-gradient(135deg, rgba(99,102,241,0.22), rgba(168,85,247,0.14))',
        border: '1.5px solid rgba(99,102,241,0.32)',
      }}
    >
      <svg className="w-8 h-8" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.6}>
        <path strokeLinecap="round" strokeLinejoin="round"
          d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
      </svg>
    </div>

    {/* Dotted connection line */}
    <div className="flex-1 h-px mx-3 relative">
      <div
        className="w-full h-px"
        style={{
          backgroundImage:
            'linear-gradient(90deg, rgba(167,139,250,0.5) 50%, transparent 50%)',
          backgroundSize: '8px 1px',
        }}
      />
      {/* Traveling cursor */}
      <motion.div
        className="absolute top-1/2 -translate-y-1/2"
        animate={{ x: ['0%', '100%', '100%', '0%', '0%'] }}
        transition={{
          duration: 4,
          times: [0, 0.4, 0.5, 0.9, 1],
          repeat: Infinity,
          ease: 'easeInOut',
        }}
      >
        <svg className="w-4 h-4 -ml-2" viewBox="0 0 24 24" fill="#a78bfa" stroke="white" strokeWidth={1.2}>
          <path d="M4 2l14 8-6 2-2 6z" />
        </svg>
      </motion.div>
    </div>

    {/* Right device */}
    <div
      className="w-24 h-20 rounded-xl flex items-center justify-center flex-shrink-0"
      style={{
        background: 'linear-gradient(135deg, rgba(168,85,247,0.22), rgba(99,102,241,0.14))',
        border: '1.5px solid rgba(168,85,247,0.32)',
      }}
    >
      <svg className="w-8 h-8" fill="none" viewBox="0 0 24 24" stroke="rgba(233,213,255,0.9)" strokeWidth={1.6}>
        <path strokeLinecap="round" strokeLinejoin="round"
          d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
      </svg>
    </div>
  </div>
);

const PairingVisual = () => (
  <div className="flex items-center justify-center gap-3 py-3">
    {[0, 1].map((i) => (
      <div
        key={i}
        className="rounded-2xl px-4 py-3 flex flex-col items-center gap-1.5"
        style={{
          background: 'var(--accent-soft-bg)',
          border: '1px solid var(--accent-soft-br)',
        }}
      >
        <p className="text-[9px] font-bold uppercase tracking-widest" style={{ color: 'var(--accent-muted)' }}>
          Device {i + 1}
        </p>
        <p
          className="font-mono font-bold tracking-[0.25em] text-xl"
          style={{ color: 'var(--text-primary)' }}
        >
          482-391
        </p>
      </div>
    ))}
  </div>
);

const PermissionsGuidance = ({ platform }: { platform: Platform }) => {
  const bodyStyle = { color: 'var(--text-secondary)' } as const;
  const mutedStyle = { color: 'var(--text-muted)' } as const;
  const emphStyle = { color: 'var(--text-primary)' } as const;

  if (platform === 'macos') {
    return (
      <>
        <p className="text-sm leading-relaxed text-center" style={bodyStyle}>
          MultiMouse needs <span className="font-semibold" style={emphStyle}>Accessibility</span> permission
          to forward mouse and keyboard events.
        </p>
        <p className="text-[11px] mt-2 text-center leading-relaxed" style={mutedStyle}>
          System Settings → Privacy &amp; Security → Accessibility → enable MultiMouse
        </p>
      </>
    );
  }
  if (platform === 'windows') {
    return (
      <>
        <p className="text-sm leading-relaxed text-center" style={bodyStyle}>
          On Windows, MultiMouse may prompt for <span className="font-semibold" style={emphStyle}>UAC elevation</span> to
          inject input into elevated apps.
        </p>
        <p className="text-[11px] mt-2 text-center leading-relaxed" style={mutedStyle}>
          Accept the prompt when it appears. Firewall: allow local network access.
        </p>
      </>
    );
  }
  if (platform === 'linux') {
    return (
      <>
        <p className="text-sm leading-relaxed text-center" style={bodyStyle}>
          On Linux, MultiMouse requires access to <span className="font-semibold" style={emphStyle}>uinput</span> or
          X11 input.
        </p>
        <p className="text-[11px] mt-2 text-center leading-relaxed" style={mutedStyle}>
          You may need to add your user to the <span className="font-mono">input</span> group
          and re-login, or run with appropriate permissions.
        </p>
      </>
    );
  }
  return (
    <>
      <p className="text-sm leading-relaxed text-center" style={bodyStyle}>
        MultiMouse needs permission to capture mouse &amp; keyboard input and forward it to other devices.
      </p>
      <p className="text-[11px] mt-2 text-center leading-relaxed" style={mutedStyle}>
        If prompted, allow accessibility / input access in your OS settings.
      </p>
    </>
  );
};

interface SlideContent {
  title: string;
  subtitle?: string;
  body: React.ReactNode;
}

export const OnboardingWizard = () => {
  const { settings, setSettings } = useStore();
  const [index, setIndex] = useState(0);
  const [direction, setDirection] = useState(1);
  const [platform, setPlatform] = useState<Platform>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    const unsub = listen<{ platform?: string }>('accessibility-needed', (e) => {
      const p = e.payload?.platform;
      if (p === 'macos' || p === 'windows' || p === 'linux') setPlatform(p);
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  const slides: SlideContent[] = [
    {
      title: 'Welcome to MultiMouse',
      subtitle: 'Share one mouse and keyboard across multiple computers',
      body: <CursorCrossAnimation />,
    },
    {
      title: 'Grant Permissions',
      body: (
        <div className="flex flex-col items-center gap-4 py-1 px-2">
          <PermissionsGuidance platform={platform} />
          {platform === 'macos' && (
            <button
              onClick={async () => {
                try {
                  const { openUrl } = await import('@tauri-apps/plugin-opener');
                  await openUrl('x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility');
                } catch (e) {
                  console.error('open system settings failed', e);
                }
              }}
              className="mt-1 px-4 py-2 rounded-xl text-xs font-semibold transition-all active:scale-95"
              style={{
                background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                color: 'white',
                boxShadow: '0 4px 14px rgba(99,102,241,0.4)',
              }}
            >
              Open System Settings
            </button>
          )}
        </div>
      ),
    },
    {
      title: 'Secure Pairing',
      subtitle: 'When you connect, both devices show the same 6-digit code. Confirm they match before accepting.',
      body: <PairingVisual />,
    },
    {
      title: "You're all set",
      subtitle: 'Your device will appear on other computers running MultiMouse on the same network.',
      body: (
        <div className="flex items-center justify-center py-4">
          <motion.div
            initial={{ scale: 0, rotate: -45 }}
            animate={{ scale: 1, rotate: 0 }}
            transition={{ type: 'spring', stiffness: 320, damping: 20 }}
            className="w-20 h-20 rounded-full flex items-center justify-center"
            style={{
              background: 'linear-gradient(135deg, #6366f1, #a855f7)',
              boxShadow: '0 8px 28px rgba(99,102,241,0.5)',
            }}
          >
            <svg className="w-10 h-10 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          </motion.div>
        </div>
      ),
    },
  ];

  const total = slides.length;
  const isLast = index === total - 1;

  const finish = async () => {
    setDismissed(true);
    const base: Settings = settings ?? ({
      transition_edge: 'right',
      hotkey_release: 'Ctrl+Ctrl',
      launch_on_startup: false,
      theme: 'dark',
      relay_url: '',
      edge_dwell_ms: 150,
      onboarding_done: false,
      auto_reconnect: true,
    } as Settings);
    const next: Settings = { ...base, onboarding_done: true };
    setSettings(next);
    try {
      await invoke('update_settings', { settings: next });
    } catch (e) {
      console.error('onboarding update_settings failed', e);
    }
  };

  const goNext = () => {
    if (isLast) { finish(); return; }
    setDirection(1);
    setIndex((i) => Math.min(i + 1, total - 1));
  };

  const goBack = () => {
    if (index === 0) return;
    setDirection(-1);
    setIndex((i) => Math.max(i - 1, 0));
  };

  if (dismissed) return null;

  const slide = slides[index];

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[60] flex items-center justify-center"
      style={{
        background: 'var(--bg-overlay)',
        backdropFilter: 'blur(14px)',
        borderRadius: '18px',
      }}
    >
      <div
        className="relative flex flex-col w-[calc(100%-32px)] max-w-[380px] rounded-3xl overflow-hidden"
        style={{
          background: 'var(--bg-card-strong)',
          border: '1px solid var(--border)',
          boxShadow: 'var(--shadow-hero)',
        }}
      >
        {/* Skip link (slides 1-3) */}
        {!isLast && (
          <button
            onClick={finish}
            className="absolute top-3 right-3 text-[11px] font-medium transition-colors z-10"
            style={{ color: 'var(--text-muted)' }}
          >
            Skip
          </button>
        )}

        {/* Slide content with directional transitions */}
        <div className="relative overflow-hidden" style={{ minHeight: 300 }}>
          <AnimatePresence initial={false} mode="wait" custom={direction}>
            <motion.div
              key={index}
              custom={direction}
              initial={{ x: direction > 0 ? 60 : -60, opacity: 0 }}
              animate={{ x: 0, opacity: 1 }}
              exit={{ x: direction > 0 ? -60 : 60, opacity: 0 }}
              transition={{ duration: 0.28, ease: 'easeOut' }}
              className="px-6 pt-10 pb-4 flex flex-col items-center"
            >
              <h2 className="text-lg font-bold text-center mb-1.5 leading-tight" style={{ color: 'var(--text-primary)' }}>
                {slide.title}
              </h2>
              {slide.subtitle && (
                <p
                  className="text-xs text-center leading-relaxed mb-4 max-w-[300px]"
                  style={{ color: 'var(--text-body)' }}
                >
                  {slide.subtitle}
                </p>
              )}
              <div className="w-full">{slide.body}</div>
            </motion.div>
          </AnimatePresence>
        </div>

        {/* Controls */}
        <div className="flex items-center justify-between px-5 py-4" style={{ borderTop: '1px solid var(--divider)' }}>
          {/* Back */}
          <button
            onClick={goBack}
            disabled={index === 0}
            className="w-9 h-9 rounded-xl flex items-center justify-center transition-all disabled:opacity-30"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-body)',
            }}
            aria-label="Back"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
            </svg>
          </button>

          {/* Dot indicator */}
          <div className="flex items-center gap-1.5">
            {slides.map((_, i) => (
              <motion.div
                key={i}
                animate={{
                  width: i === index ? 18 : 6,
                  opacity: i === index ? 1 : 0.35,
                }}
                transition={{ type: 'spring', stiffness: 420, damping: 30 }}
                className="h-1.5 rounded-full"
                style={{
                  background: i === index ? 'linear-gradient(90deg, #6366f1, #a855f7)' : 'var(--text-faint)',
                }}
              />
            ))}
          </div>

          {/* Next / Done */}
          {isLast ? (
            <button
              onClick={finish}
              className="px-4 h-9 rounded-xl text-sm font-semibold transition-all active:scale-95"
              style={{
                background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                color: 'white',
                boxShadow: '0 4px 14px rgba(99,102,241,0.4)',
              }}
            >
              Done
            </button>
          ) : (
            <button
              onClick={goNext}
              className="w-9 h-9 rounded-xl flex items-center justify-center transition-all active:scale-95"
              style={{
                background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                color: 'white',
                boxShadow: '0 4px 14px rgba(99,102,241,0.4)',
              }}
              aria-label="Next"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </motion.div>
  );
};
