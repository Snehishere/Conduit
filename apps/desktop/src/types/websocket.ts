// GENERATED FILE - DO NOT EDIT
// Generated from packages/protocol/schema.json by scripts/generate_types.js
// Run `npm run generate` to refresh.

export interface DeviceInfo {
  name: string;
  type: 'phone' | 'tablet' | 'desktop';
  os?: 'ios' | 'android' | 'windows' | 'macos' | 'linux' | 'unknown';
  battery?: number;
}

export interface AutomationTrigger {
  type: 'device_connect' | 'device_disconnect' | 'time' | 'battery_level' | 'wifi_change' | 'app_open' | 'audio_device_connect' | 'audio_device_disconnect';
  device_id?: string;
  time?: string;
  below?: number;
  ssid?: string;
  app_package?: string;
}

export interface AutomationActionPayload {
  type: 'send_notification' | 'set_phone_profile' | 'route_audio' | 'run_shell_command' | 'toggle_wifi' | 'toggle_bluetooth' | 'open_url' | 'open_app' | 'set_window_state';
  title?: string;
  body?: string;
  profile?: 'silent' | 'vibrate' | 'ring';
  device_id?: string;
  command?: string;
  enabled?: boolean;
  url?: string;
  app_package?: string;
  state?: 'minimize' | 'maximize' | 'restore' | 'close';
}

export interface SmsMessage {
  id: string;
  address: string;
  body: string;
  timestamp: number;
  read: boolean;
  is_outgoing: boolean;
}

export interface SmsThread {
  thread_id: string;
  address: string;
  name?: string | null;
  snippet: string;
  unread_count: number;
  timestamp: number;
  messages: Array<SmsMessage>;
}

export interface DiscoveryAnnounceMessage {
  type: 'discovery';
  action: 'announce';
  protocol_version?: number;
  device_id: string;
  device_name: string;
  device_type: 'phone' | 'tablet' | 'desktop';
  os?: string;
  version?: string;
  battery?: number;
  ws_port?: number;
  wss_port?: number;
  apns_token?: string;
}

export interface DiscoveryRemoveMessage {
  type: 'discovery';
  action: 'remove';
  device_id: string;
}

export interface PairingRequestMessage {
  type: 'pairing';
  action: 'request';
  protocol_version?: number;
  token: string;
  public_key: string;
  device_info?: DeviceInfo;
}

export interface PairingAcceptMessage {
  type: 'pairing';
  action: 'accept';
  protocol_version?: number;
  public_key: string;
  device_info?: DeviceInfo;
  device_id?: string;
  hub_device_id?: string;
  relay_url?: string;
  relay_token?: string;
  relay_cert_pin?: string;
}

export interface PairingRevokeMessage {
  type: 'pairing';
  action: 'revoke';
  protocol_version?: number;
  reason?: string;
}

export interface ClipboardSyncMessage {
  type: 'clipboard';
  action: 'sync';
  content: string;
  mime: string;
  source_device: string;
  timestamp: number;
}

export interface ClipboardRequestMessage {
  type: 'clipboard';
  action: 'request';
  mime?: string;
}

export interface NotificationPostMessage {
  type: 'notification';
  action: 'post';
  id: string;
  device_id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  actions?: Array<string>;
}

export interface NotificationDismissMessage {
  type: 'notification';
  action: 'dismiss';
  id: string;
}

export interface NotificationReplyMessage {
  type: 'notification';
  action: 'reply';
  id: string;
  text: string;
}

export interface NotificationMarkReadMessage {
  type: 'notification';
  action: 'mark_read';
}

export interface FileRequestMessage {
  type: 'file';
  action: 'request';
  id: string;
  name: string;
  size: number;
  mime: string;
  from: string;
  to?: string;
  checksum?: string;
}

export interface FileAcceptMessage {
  type: 'file';
  action: 'accept';
  id: string;
}

export interface FileChunkMessage {
  type: 'file';
  action: 'chunk';
  id: string;
  index: number;
  data: string;
  total?: number;
}

export interface FileProgressMessage {
  type: 'file';
  action: 'progress';
  id: string;
  percent: number;
}

export interface FileCompleteMessage {
  type: 'file';
  action: 'complete';
  id: string;
  path?: string;
}

export interface FileCancelMessage {
  type: 'file';
  action: 'cancel';
  id: string;
}

export interface FileResumeMessage {
  type: 'file';
  action: 'resume';
  id: string;
  name: string;
  size: number;
  mime: string;
  from: string;
  checksum?: string;
}

export interface FileResumeAckMessage {
  type: 'file';
  action: 'resume_ack';
  id: string;
  chunks_loaded: number;
}

export interface AudioStreamStartMessage {
  type: 'audio';
  action: 'stream_start';
}

export interface AudioStreamStopMessage {
  type: 'audio';
  action: 'stream_stop';
}

export interface AudioStreamStartedMessage {
  type: 'audio';
  action: 'stream_started';
  from: string;
}

export interface AudioStreamDataMessage {
  type: 'audio';
  action: 'stream_data';
  data: string;
  format: 'pcm16';
  sample_rate: number;
  channels: number;
  from: string;
}

export interface AudioPlaybackStartMessage {
  type: 'audio';
  action: 'playback_start';
}

export interface AudioPlaybackStopMessage {
  type: 'audio';
  action: 'playback_stop';
}

export interface AudioPlaybackStartedMessage {
  type: 'audio';
  action: 'playback_started';
  from: string;
}

export interface AudioPlaybackDataMessage {
  type: 'audio';
  action: 'playback_data';
  data: string;
  format: 'pcm16';
  sample_rate: number;
  channels: number;
}

export interface ScreenMirrorStartMessage {
  type: 'screen_mirror';
  action: 'start';
  quality?: 'low' | 'medium' | 'high';
}

export interface ScreenMirrorStopMessage {
  type: 'screen_mirror';
  action: 'stop';
}

export interface ScreenMirrorFrameMessage {
  type: 'screen_mirror';
  action: 'frame';
  data: string;
  format: 'jpeg';
  width: number;
  height: number;
  from_desktop?: boolean;
}

export interface ScreenMirrorCaptureStoppedMessage {
  type: 'screen_mirror';
  action: 'capture_stopped';
}

export interface ScreenMirrorTouchMessage {
  type: 'screen_mirror';
  action: 'touch';
  x: number;
  y: number;
  actionType?: 'tap' | 'double_tap' | 'long_press' | 'right_click';
}

export interface ScreenMirrorKeyMessage {
  type: 'screen_mirror';
  action: 'key';
  key: string;
  modifiers?: Array<'shift' | 'control' | 'alt' | 'meta'>;
}

export interface ScreenMirrorScrollMessage {
  type: 'screen_mirror';
  action: 'scroll';
  dx: number;
  dy: number;
}

export interface RemoteInputMoveMessage {
  type: 'remote_input';
  action: 'move';
  dx: number;
  dy: number;
}

export interface RemoteInputClickMessage {
  type: 'remote_input';
  action: 'click';
  button: 'left' | 'right' | 'double_left' | 'middle';
}

export interface RemoteInputScrollMessage {
  type: 'remote_input';
  action: 'scroll';
  dx: number;
  dy: number;
}

export interface AutomationRuleMessage {
  type: 'automation';
  action: 'rule';
  id: string;
  name: string;
  trigger: AutomationTrigger;
  rule_action: AutomationActionPayload;
  enabled: boolean;
}

export type AutomationDeleteMessage = {
  type: 'automation';
  action: 'delete';
  rule_id?: string;
  id?: string;
} & (
  | {
    type?: 'automation';
    action?: 'delete';
    rule_id: string;
    id?: string;
  }
  | {
    type?: 'automation';
    action?: 'delete';
    rule_id?: string;
    id: string;
  }
);

export interface AutomationSyncMessage {
  type: 'automation';
  action: 'sync';
  rules: Array<any>;
  full_sync?: boolean;
}

export interface AutomationTriggeredMessage {
  type: 'automation';
  action: 'triggered';
  id: string;
  trigger_type: string;
  device_id?: string;
}

export interface CallMessageMessage {
  type: 'call';
  action: string;
  call_id?: string;
  to_device_id?: string;
  route?: string;
  number?: string;
  name?: string;
  device_id?: string;
}

export interface SMSSendMessage {
  type: 'sms';
  action: 'send';
  to: string;
  body: string;
}

export interface StatusUpdateMessage {
  type: 'status';
  action: 'update';
  battery?: number;
  wifi_ssid?: string;
  device_info?: DeviceInfo;
}

export interface PingMessage {
  type: 'ping';
}

export interface PongMessage {
  type: 'pong';
}

export interface ErrorMessage {
  type: 'error';
  code: string;
  message: string;
  server_version?: number;
}

export interface RelayAuthMessage {
  type: 'relay_auth';
  device_id: string;
  relay_token: string;
  apns_token?: string;
}

export interface RelayAuthOKMessage {
  type: 'relay_auth_ok';
}

export interface RelayAuthRejectedMessage {
  type: 'relay_auth_rejected';
  reason: 'invalid_token' | 'missing_token';
}

export interface RelayRouteMessage {
  type: 'relay_route';
  to_device_id: string;
  payload: any;
}

export interface RelayDeliveryMessage {
  type: 'relay_delivery';
  from_device_id: string;
  to_device_id?: string;
  payload: any;
}

export interface EncryptedEnvelopeMessage {
  type: 'encrypted';
  nonce: string;
  hmac: string;
  data: string;
  source_device?: string;
  protocol_version?: number;
}

export interface SMSSyncMessage {
  type: 'sms';
  action: 'sync';
  threads: Array<SmsThread>;
}

export type SMSNewMessage = {
  type: 'sms';
  action: 'new';
  from?: string;
  body?: string;
  timestamp?: number;
  thread_id?: string;
  message?: SmsMessage;
} & (
  | {
    type?: 'sms';
    action?: 'new';
    from: string;
    body: string;
    timestamp: number;
    thread_id?: string;
    message?: SmsMessage;
  }
  | {
    type?: 'sms';
    action?: 'new';
    from?: string;
    body?: string;
    timestamp?: number;
    thread_id: string;
    message: SmsMessage;
  }
);

export type SMSSentMessage = {
  type: 'sms';
  action: 'sent';
  to?: string;
  body?: string;
  timestamp?: number;
  thread_id?: string;
  message?: SmsMessage;
} & (
  | {
    type?: 'sms';
    action?: 'sent';
    to: string;
    body: string;
    timestamp: number;
    thread_id?: string;
    message?: SmsMessage;
  }
  | {
    type?: 'sms';
    action?: 'sent';
    to?: string;
    body?: string;
    timestamp?: number;
    thread_id: string;
    message: SmsMessage;
  }
);

export type WebSocketMessage =
  | DiscoveryAnnounceMessage
  | DiscoveryRemoveMessage
  | PairingRequestMessage
  | PairingAcceptMessage
  | PairingRevokeMessage
  | ClipboardSyncMessage
  | ClipboardRequestMessage
  | NotificationPostMessage
  | NotificationDismissMessage
  | NotificationReplyMessage
  | NotificationMarkReadMessage
  | FileRequestMessage
  | FileAcceptMessage
  | FileChunkMessage
  | FileProgressMessage
  | FileCompleteMessage
  | FileCancelMessage
  | FileResumeMessage
  | FileResumeAckMessage
  | AudioStreamStartMessage
  | AudioStreamStopMessage
  | AudioStreamStartedMessage
  | AudioStreamDataMessage
  | AudioPlaybackStartMessage
  | AudioPlaybackStopMessage
  | AudioPlaybackStartedMessage
  | AudioPlaybackDataMessage
  | ScreenMirrorStartMessage
  | ScreenMirrorStopMessage
  | ScreenMirrorFrameMessage
  | ScreenMirrorCaptureStoppedMessage
  | ScreenMirrorTouchMessage
  | ScreenMirrorKeyMessage
  | ScreenMirrorScrollMessage
  | RemoteInputMoveMessage
  | RemoteInputClickMessage
  | RemoteInputScrollMessage
  | AutomationRuleMessage
  | AutomationDeleteMessage
  | AutomationSyncMessage
  | AutomationTriggeredMessage
  | CallMessageMessage
  | SMSSendMessage
  | StatusUpdateMessage
  | PingMessage
  | PongMessage
  | ErrorMessage
  | RelayAuthMessage
  | RelayAuthOKMessage
  | RelayAuthRejectedMessage
  | RelayRouteMessage
  | RelayDeliveryMessage
  | EncryptedEnvelopeMessage
  | SMSSyncMessage
  | SMSNewMessage
  | SMSSentMessage;

export type MessageHandler = (data: WebSocketMessage) => void;
