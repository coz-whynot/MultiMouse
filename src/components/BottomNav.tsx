import { motion } from 'framer-motion';

type Page = 'home' | 'layout' | 'settings';

interface Props {
  current: Page;
  onChange: (page: Page) => void;
  connectedPeer?: boolean;
}

const HomeIcon = () => (
  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
  </svg>
);

const LayoutIcon = () => (
  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M9 17V7m0 10a2 2 0 01-2 2H5a2 2 0 01-2-2V7a2 2 0 012-2h2a2 2 0 012 2m0 10a2 2 0 002 2h2a2 2 0 002-2M9 7a2 2 0 012-2h2a2 2 0 012 2m0 10V7m0 10a2 2 0 002 2h2a2 2 0 002-2V7a2 2 0 00-2-2h-2a2 2 0 00-2 2" />
  </svg>
);

const SettingsIcon = () => (
  <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
    <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
  </svg>
);

const TABS: { id: Page; label: string; Icon: React.FC }[] = [
  { id: 'home', label: 'Devices', Icon: HomeIcon },
  { id: 'layout', label: 'Layout', Icon: LayoutIcon },
  { id: 'settings', label: 'Settings', Icon: SettingsIcon },
];

export const BottomNav = ({ current, onChange, connectedPeer }: Props) => (
  <div
    className="flex-shrink-0 flex px-2 pb-2 pt-1 gap-1"
    style={{ borderTop: '1px solid rgba(255,255,255,0.07)' }}
  >
    {TABS.map(({ id, label, Icon }) => {
      const active = current === id;
      const badge = id === 'layout' && connectedPeer;
      return (
        <button
          key={id}
          onClick={() => onChange(id)}
          className="flex-1 flex flex-col items-center gap-1 py-2 rounded-xl transition-all relative"
          style={{
            background: active ? 'rgba(99,102,241,0.15)' : 'transparent',
            color: active ? '#a78bfa' : 'rgba(255,255,255,0.3)',
          }}
        >
          {badge && (
            <motion.div
              animate={{ scale: [1, 1.3, 1] }}
              transition={{ repeat: Infinity, duration: 2 }}
              className="absolute top-1.5 right-4 w-1.5 h-1.5 rounded-full bg-emerald-400"
            />
          )}
          <Icon />
          <span className="text-[9px] font-semibold uppercase tracking-wider">{label}</span>
          {active && (
            <motion.div
              layoutId="nav-pill"
              className="absolute bottom-1 w-4 h-0.5 rounded-full"
              style={{ background: '#a78bfa' }}
            />
          )}
        </button>
      );
    })}
  </div>
);
