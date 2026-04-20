export interface PeerInfo {
  id: string;
  name: string;
  addr: string;
  port: number;
  status: 'available' | 'connected' | 'pairing' | 'error';
  ping_ms?: number;
  is_known: boolean;
  /** Peer's advertised app version from mDNS TXT. Missing if peer is on an
   *  older build that didn't advertise it. */
  app_version?: string | null;
}

export interface ScreenSize {
  width: number;
  height: number;
}

export interface AppStatus {
  device_id: string;
  device_name: string;
  connected_peer: string | null;
  relaying: boolean;
  is_controlled: boolean;
  local_screen?: ScreenSize;
  remote_screen?: ScreenSize | null;
}

export interface Settings {
  transition_edge: 'left' | 'right' | 'top' | 'bottom';
  hotkey_release: string;
  launch_on_startup: boolean;
  theme: 'dark' | 'light';
  relay_url: string;
  edge_dwell_ms: number;
  onboarding_done: boolean;
  auto_reconnect: boolean;
  idle_lock_minutes: number;
  privacy_blur_on_relay: boolean;
  gaming_mode: boolean;
  hide_cursor_during_relay: boolean;
  auto_gaming_mode: boolean;
  /** Swap-control hotkeys (rdev Debug-format key names, e.g. "F9"). Only
   *  active while `gaming_mode` is true. Up to 9 entries supported. */
  switch_hotkeys: string[];
  /** Multiplier applied to sender-side mouse deltas. 1.0 = native feel.
   *  Range [0.1, 5.0] — clamped on the Rust side. */
  mouse_sensitivity: number;
  /** v0.3.11+ — reveals Settings → Developer and enables cross-PC diag
   *  sync. Off by default. */
  developer_mode: boolean;
  /** v0.3.18+ — when false, suppresses the peer-version-newer banner
   *  and the Settings → About manual update check. Default true. */
  updates_enabled: boolean;
}

export interface PairingRequest {
  peer_id: string;
  peer_name: string;
  pin: string;
}

export interface TransferInfo {
  id: string;
  name: string;
  size: number;
  transferred: number;
  direction: 'sending' | 'receiving';
  peer_id: string;
  peer_name: string;
  status: 'pending' | 'active' | 'done' | 'error' | 'rejected';
}

export interface KnownDevice {
  id: string;
  name: string;
  addr: string;
  port: number;
  session_key: string;
}

export interface FileOffer {
  id: string;
  name: string;
  size: number;
  peer_id: string;
  peer_name: string;
}

export interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale_factor: number;
  is_primary: boolean;
}

export interface BandwidthStats {
  bytes_sent: number;
  bytes_received: number;
  uptime_secs: number;
}

export interface AuditEntry {
  timestamp: number; // unix seconds
  action: 'connected' | 'disconnected' | 'pairing_rejected' | string;
  peer_name: string;
  peer_id?: string;
  details?: string;
}

export interface IncomingDeepLink {
  code: string;
  host: string;
}
