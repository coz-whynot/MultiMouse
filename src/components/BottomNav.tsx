import { motion } from 'framer-motion';

type Page = 'home' | 'layout' | 'settings';

interface Props {
  current: Page;
  onChange: (page: Page) => void;
  connectedPeer?: boolean;
}

const HomeIcon = ({ active }: { active: boolean }) => (
  <svg className="w-[22px] h-[22px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={active ? 2.2 : 1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
  </svg>
);

const LayoutIcon = ({ active }: { active: boolean }) => (
  <svg className="w-[22px] h-[22px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={active ? 2.2 : 1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
  </svg>
);

const SettingsIcon = ({ active }: { active: boolean }) => (
  <svg className="w-[22px] h-[22px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={active ? 2.2 : 1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
    <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
  </svg>
);

const TABS: { id: Page; label: string }[] = [
  { id: 'home', label: 'Devices' },
  { id: 'layout', label: 'Control' },
  { id: 'settings', label: 'Settings' },
];

export const BottomNav = ({ current, onChange, connectedPeer }: Props) => (
  <div
    className="flex-shrink-0 flex px-3 pb-3 pt-2 gap-1"
    style={{ borderTop: '1px solid rgba(255,255,255,0.07)' }}
  >
    {TABS.map(({ id, label }) => {
      const active = current === id;
      const hasBadge = id === 'layout' && connectedPeer;

      return (
        <button
          key={id}
          onClick={() => onChange(id)}
          className="flex-1 flex flex-col items-center gap-1.5 py-2 rounded-2xl transition-colors duration-150 relative"
          style={{
            background: active ? 'rgba(99,102,241,0.14)' : 'transparent',
          }}
        >
          {/* Live badge */}
          {hasBadge && (
            <motion.div
              animate={{ scale: [1, 1.4, 1], opacity: [1, 0.5, 1] }}
              transition={{ repeat: Infinity, duration: 2.2 }}
              className="absolute top-2 right-[calc(50%-18px)] w-2 h-2 rounded-full"
              style={{ background: '#34d399', boxShadow: '0 0 6px #34d39966' }}
            />
          )}

          <motion.span
            animate={{ color: active ? '#a78bfa' : 'rgba(255,255,255,0.32)' }}
            transition={{ duration: 0.15 }}
          >
            {id === 'home' && <HomeIcon active={active} />}
            {id === 'layout' && <LayoutIcon active={active} />}
            {id === 'settings' && <SettingsIcon active={active} />}
          </motion.span>

          <motion.span
            className="text-[9px] font-semibold uppercase tracking-wider"
            animate={{ color: active ? '#a78bfa' : 'rgba(255,255,255,0.32)' }}
            transition={{ duration: 0.15 }}
          >
            {label}
          </motion.span>

          {/* Active underline pill — shared layout animation */}
          {active && (
            <motion.div
              layoutId="nav-pill"
              className="absolute bottom-1.5 rounded-full"
              style={{
                height: 2,
                width: 20,
                background: 'linear-gradient(90deg, #6366f1, #a855f7)',
              }}
              transition={{ type: 'spring', stiffness: 500, damping: 35 }}
            />
          )}
        </button>
      );
    })}
  </div>
);
