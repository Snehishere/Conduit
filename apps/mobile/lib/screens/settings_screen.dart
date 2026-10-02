import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../services/websocket_service.dart';
import '../services/clipboard_service.dart';
import '../services/file_service.dart';
import '../theme/app_theme.dart';
import '../theme/theme_provider.dart';

class SettingsScreen extends StatefulWidget {
  final bool embedded;

  const SettingsScreen({super.key, this.embedded = false});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  bool _syncNotifications = true;
  bool _syncClipboard = true;
  bool _autoClipboardSync = true;
  bool _autoAnswerCalls = false;
  bool _crashReports = true;
  bool _initialized = false;

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _saveSetting(String key, bool value) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(key, value);
  }

  final _relayUrlController = TextEditingController();
  final _relayTokenController = TextEditingController();

  Future<void> _loadSettings() async {
    // Read the service before the first await, so no `BuildContext` is used
    // across an async gap, and the relay values come from the service rather
    // than from these prefs — the desktop delivers them in `pairing/accept` and
    // they are stored as the credential the token is.
    final ws = context.read<WebSocketService>();
    final prefs = await SharedPreferences.getInstance();
    await ws.ensureRelayConfigLoaded();
    if (!mounted) return;

    setState(() {
      _syncNotifications = prefs.getBool('sync_notifications') ?? true;
      _syncClipboard = prefs.getBool('sync_clipboard') ?? true;
      _autoClipboardSync = prefs.getBool('auto_clipboard_sync') ?? true;
      _autoAnswerCalls = prefs.getBool('auto_answer_calls') ?? false;
      _crashReports = prefs.getBool('crash_reports') ?? true;
      _relayUrlController.text = ws.relayUrl ?? '';
      _relayTokenController.text = ws.relayToken ?? '';
      _initialized = true;
    });

    _adoptLegacyRelayPrefs(prefs, ws);
  }

  /// One-way move of any relay values a build that *had* a Relay section wrote
  /// into preferences.
  ///
  /// Those keys were only ever written by a function nothing could reach, so
  /// this is close to a formality; it is here so a device that does hold such a
  /// value does not silently lose its only relay configuration the first time
  /// it opens this screen after an upgrade. The service is the only place the
  /// relay is stored from here on, so the old keys are read once and never
  /// written again.
  ///
  /// Each field is adopted on its own: a stale address must never displace a
  /// token that is already right, or the other way round.
  void _adoptLegacyRelayPrefs(SharedPreferences prefs, WebSocketService ws) {
    if (ws.relayUrl == null) ws.setRelayConfig(prefs.getString('relay_url'));
    if (ws.relayToken == null) ws.setRelayToken(prefs.getString('relay_token'));
  }

  /// Persist the relay URL/token and push them into [WebSocketService].
  ///
  /// This is the manual path, and an override: the desktop normally hands both
  /// values over in `pairing/accept`, so pairing alone is enough and there is
  /// nothing to type here. It exists for the case pairing cannot cover — a relay
  /// the desktop does not host, such as an operator's own behind a tunnel.
  ///
  /// Empty fields clear the value rather than storing an empty string, so
  /// "fall back to the hub" is a reachable state.
  Future<void> _saveRelaySettings() async {
    final ws = context.read<WebSocketService>();
    ws.setRelayConfig(_relayUrlController.text);
    ws.setRelayToken(_relayTokenController.text);

    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Relay settings saved')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final content = _buildBody();
    if (widget.embedded) return content;

    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: content,
    );
  }

  Widget _buildBody() {
    final colors = Theme.of(context).extension<AppColors>()!;
    final themeProvider = context.watch<ThemeProvider>();

    return ListView(
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        // ── Device ──
        _buildSection('Device', colors, [
          Consumer<WebSocketService>(
            builder: (_, ws, _) => ListTile(
              leading: Icon(Icons.phone_android, color: colors.accent),
              title: Text('This device', style: TextStyle(color: colors.text1)),
              subtitle: Text(
                ws.isConnected ? 'Connected' : 'Disconnected',
                style: TextStyle(color: colors.text2, fontSize: 12),
              ),
              trailing: Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: ws.isConnected ? colors.success : colors.error,
                ),
              ),
            ),
          ),
          Consumer<WebSocketService>(
            builder: (_, ws, _) {
              // The endpoint this app is actually connected to, e.g.
              // `wss://192.168.1.5:9531`, or the relay URL when connected
              // through a relay. Never synthesised — a hardcoded placeholder
              // would be indistinguishable from a real connection and would
              // name the wrong port for a TLS session.
              final address = ws.connectedAddress;
              return ListTile(
                leading: Icon(Icons.wifi, color: colors.warning),
                title: Text('Hub Address', style: TextStyle(color: colors.text1)),
                subtitle: Text(
                  address ?? 'Not connected',
                  style: TextStyle(color: colors.text2, fontSize: 12),
                ),
              );
            },
          ),
        ]),

        // ── Appearance ──
        _buildSection('Appearance', colors, [
          // Theme mode
          ListTile(
            leading: Icon(Icons.dark_mode, color: colors.accentSecondary),
            title: Text('Theme', style: TextStyle(color: colors.text1)),
            subtitle: Text(
              _themeModeLabel(themeProvider.themeMode),
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            trailing: SegmentedButton<ThemeMode>(
              segments: const [
                ButtonSegment(value: ThemeMode.dark, icon: Icon(Icons.dark_mode, size: 16)),
                ButtonSegment(value: ThemeMode.light, icon: Icon(Icons.light_mode, size: 16)),
                ButtonSegment(value: ThemeMode.system, icon: Icon(Icons.brightness_auto, size: 16)),
              ],
              selected: {themeProvider.themeMode},
              onSelectionChanged: (modes) => themeProvider.setThemeMode(modes.first),
              style: const ButtonStyle(
                visualDensity: VisualDensity.compact,
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
            ),
          ),
          // Accent color
          ListTile(
            leading: Icon(Icons.palette, color: colors.accent),
            title: Text('Accent color', style: TextStyle(color: colors.text1)),
            subtitle: Text(
              'Choose your accent color',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Wrap(
              spacing: 10,
              runSpacing: 10,
              children: accentColorPresets.entries.map((entry) {
                final isSelected = themeProvider.accentColor == entry.value;
                return GestureDetector(
                  onTap: () => themeProvider.setAccentColor(entry.value),
                  child: AnimatedContainer(
                    duration: const Duration(milliseconds: 200),
                    width: 32,
                    height: 32,
                    decoration: BoxDecoration(
                      color: entry.value,
                      shape: BoxShape.circle,
                      border: isSelected
                          ? Border.all(color: colors.text1, width: 2.5)
                          : null,
                    ),
                    child: isSelected
                        ? Icon(Icons.check, size: 16, color: colors.bg0)
                        : null,
                  ),
                );
              }).toList(),
            ),
          ),
          const SizedBox(height: 16),
        ]),

        // ── Sync ──
        // `sync_notifications`, `sync_clipboard` and `auto_answer_calls` are
        // persisted but nothing reads them, so those switches change nothing
        // today. The subtitles say so rather than implying the sync is being
        // suppressed. Wire them up, or drop the rows, before release.
        _buildSection('Sync', colors, [
          SwitchListTile(
            secondary: Icon(Icons.notifications_active, color: colors.accentSecondary),
            title: Text(
              'Sync notifications',
              style: TextStyle(color: colors.text1),
            ),
            subtitle: Text(
              'Forward this phone\'s notifications to your desktop.\n'
              'Always on for now — this switch has no effect.',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            value: _syncNotifications,
            onChanged: _initialized
                ? (v) {
                    setState(() => _syncNotifications = v);
                    _saveSetting('sync_notifications', v);
                  }
                : null,
          ),
          SwitchListTile(
            secondary: Icon(Icons.content_paste, color: colors.accent),
            title: Text(
              'Sync clipboard',
              style: TextStyle(color: colors.text1),
            ),
            subtitle: Text(
              'Share clipboard contents with your desktop.\n'
              'Always on for now — this switch has no effect.',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            value: _syncClipboard,
            onChanged: _initialized
                ? (v) {
                    setState(() => _syncClipboard = v);
                    _saveSetting('sync_clipboard', v);
                  }
                : null,
          ),
          SwitchListTile(
            secondary: Icon(Icons.sync, color: colors.accentSecondary),
            title: Text(
              'Auto-sync clipboard',
              style: TextStyle(color: colors.text1),
            ),
            subtitle: Text(
              'Watch the clipboard and send changes every two seconds',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            value: _autoClipboardSync,
            onChanged: _initialized
                ? (v) {
                    setState(() => _autoClipboardSync = v);
                    _saveSetting('auto_clipboard_sync', v);
                    final clipboard = context.read<ClipboardService>();
                    if (v) {
                      clipboard.startListening();
                    } else {
                      clipboard.stopListening();
                    }
                  }
                : null,
          ),
        ]),

        // ── Calls ──
        _buildSection('Calls', colors, [
          SwitchListTile(
            secondary: Icon(Icons.phone, color: colors.accent),
            title: Text(
              'Auto-answer on desktop',
              style: TextStyle(color: colors.text1),
            ),
            subtitle: Text(
              'Offer incoming calls to your desktop automatically.\n'
              'Not connected yet — forward a call from the Calls screen.',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            value: _autoAnswerCalls,
            onChanged: _initialized
                ? (v) {
                    setState(() => _autoAnswerCalls = v);
                    _saveSetting('auto_answer_calls', v);
                  }
                : null,
          ),
        ]),

        // ── Diagnostics ──
        _buildSection('Diagnostics', colors, [
          SwitchListTile(
            secondary: Icon(Icons.shield, color: colors.warning),
            title: Text('Crash reports', style: TextStyle(color: colors.text1)),
            subtitle: Text(
              'No diagnostic data is collected or sent, so this switch '
              'has no effect.',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            value: _crashReports,
            onChanged: _initialized
                ? (v) {
                    setState(() => _crashReports = v);
                    _saveSetting('crash_reports', v);
                  }
                : null,
          ),
        ]),

        // ── Relay ──
        // The relay is the path to the hub when this phone is not on its
        // network. Its address and token normally arrive in `pairing/accept`,
        // so this section only has to *show* what pairing configured and offer
        // an override for a relay the desktop does not host.
        _buildSection('Relay', colors, [
          Consumer<WebSocketService>(
            builder: (_, ws, _) => ListTile(
              leading: Icon(Icons.hub, color: colors.accent),
              title: Text('Relay', style: TextStyle(color: colors.text1)),
              subtitle: Text(
                ws.isRelayConnection
                    ? 'Connected through the relay'
                    : ws.relayUrl ??
                        'Not configured — pair with a desktop hosting one',
                style: TextStyle(color: colors.text2, fontSize: 12),
              ),
              trailing: Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: ws.isRelayConnection ? colors.success : colors.text3,
                ),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
            child: TextField(
              controller: _relayUrlController,
              enabled: _initialized,
              autocorrect: false,
              keyboardType: TextInputType.url,
              style: TextStyle(color: colors.text1, fontSize: 13),
              decoration: InputDecoration(
                labelText: 'Relay address',
                hintText: 'wss://relay.example.com:9529',
                labelStyle: TextStyle(color: colors.text3),
                hintStyle: TextStyle(color: colors.text3),
                border: const OutlineInputBorder(),
                isDense: true,
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: TextField(
              controller: _relayTokenController,
              enabled: _initialized,
              autocorrect: false,
              obscureText: true,
              style: TextStyle(color: colors.text1, fontSize: 13),
              decoration: InputDecoration(
                labelText: 'Relay token',
                labelStyle: TextStyle(color: colors.text3),
                border: const OutlineInputBorder(),
                isDense: true,
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
            child: Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                onPressed: _initialized ? _saveRelaySettings : null,
                child: const Text('Save relay settings'),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
            child: Text(
              'Leave both blank to reach the desktop only on this network. '
              'Pairing normally fills these in for you.',
              style: TextStyle(color: colors.text3, fontSize: 11),
            ),
          ),
        ]),

        // ── Storage ──
        _buildSection('Storage', colors, [
          ListTile(
            leading: Icon(Icons.folder, color: colors.accent),
            title: Text(
              'Clear file cache',
              style: TextStyle(color: colors.text1),
            ),
            subtitle: Text(
              'Delete received files and their transfer records',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            onTap: () => _showClearCacheDialog(context),
          ),
        ]),



        // ── About ──
        _buildSection('About', colors, [
          ListTile(
            leading: Icon(Icons.info_outline, color: colors.text2),
            title: Text('Version', style: TextStyle(color: colors.text1)),
            trailing: Text('0.1.0', style: TextStyle(color: colors.text3)),
          ),
          ListTile(
            leading: Icon(Icons.arrow_back, color: colors.accent),
            title: Text('Back to home', style: TextStyle(color: colors.text1)),
            subtitle: Text(
              'Close settings and return to the home screen',
              style: TextStyle(color: colors.text2, fontSize: 12),
            ),
            // There is no onboarding or tutorial flow to re-run. This row
            // used to read "Show Tutorial Again" and pop to the first route,
            // which only returned to Home — a label for a feature that does
            // not exist. It now describes what the tap actually does.
            onTap: () {
              Navigator.of(context).popUntil((route) => route.isFirst);
            },
          ),
        ]),
      ],
    );
  }

  String _themeModeLabel(ThemeMode mode) {
    switch (mode) {
      case ThemeMode.dark:
        return 'Dark';
      case ThemeMode.light:
        return 'Light';
      case ThemeMode.system:
        return 'System';
    }
  }

  Widget _buildSection(String title, AppColors colors, List<Widget> children) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 20, 16, 8),
          child: Text(
            title.toUpperCase(),
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: colors.text3,
              letterSpacing: 0.5,
            ),
          ),
        ),
        ...children,
        Divider(height: 1, color: colors.border),
      ],
    );
  }

  void _showClearCacheDialog(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('Clear file cache', style: TextStyle(color: colors.text1)),
        content: Text(
          'This deletes every received file from this device and clears '
          'the transfer history. It cannot be undone.',
          style: TextStyle(color: colors.text2, fontSize: 13),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
          ),
          TextButton(
            onPressed: () async {
              final fileService = context.read<FileService>();
              await fileService.clearCache();
              if (ctx.mounted) Navigator.pop(ctx);
              if (context.mounted) {
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('Received files deleted')),
                );
              }
            },
            child: Text('Clear', style: TextStyle(color: colors.error)),
          ),
        ],
      ),
    );
  }
}
