export interface PeerInfo {
  id: string;
  name: string;
  addr: string;
  port: number;
  status: 'available' | 'connected' | 'pairing' | 'error';
  ping_ms?: number;
  is_known: boolean;
}

export interface AppStatus {
  device_id: string;
  device_name: string;
  connected_peer: string | null;
  relaying: boolean;
}

export interface Settings {
  transition_edge: 'left' | 'right' | 'top' | 'bottom';
  hotkey_release: string;
  launch_on_startup: boolean;
  theme: 'dark' | 'light';
  relay_url: string;
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
