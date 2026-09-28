import 'package:flutter/material.dart';
import '../theme/app_theme.dart';
import '../screens/pairing_screen.dart';
import '../screens/discovery_screen.dart';
import '../screens/screen_mirror_screen.dart';
import '../screens/remote_input_screen.dart';
import '../screens/automation_rules_screen.dart';
import '../screens/clipboard_screen.dart';
import '../screens/messages_screen.dart';
import '../screens/calls_screen.dart';

/// Bottom sheet menu shown when tapping the "+" tab in the bottom nav.
/// Contains all secondary actions that don't have their own tab.
class ActionsMenuSheet extends StatelessWidget {
  final List<dynamic> connectedDevices;

  const ActionsMenuSheet({super.key, this.connectedDevices = const []});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final hasDesktop = connectedDevices.any((d) => d.type == 'desktop');

    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const SizedBox(height: 8),
          // Drag handle
          Container(
            width: 40,
            height: 4,
            decoration: BoxDecoration(
              color: colors.text3,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
          const SizedBox(height: 16),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                'Actions',
                style: TextStyle(
                  fontSize: 18,
                  fontWeight: FontWeight.bold,
                  color: colors.text1,
                ),
              ),
            ),
          ),
          const SizedBox(height: 12),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: GridView.count(
              crossAxisCount: 2,
              shrinkWrap: true,
              physics: const NeverScrollableScrollPhysics(),
              mainAxisSpacing: 8,
              crossAxisSpacing: 8,
              childAspectRatio: 2.2,
              children: [
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.qr_code_scanner_outlined,
                  title: 'Pair device',
                  subtitle: 'Scan a QR code',
                  onTap: () => _navigateTo(context, const PairingScreen()),
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.wifi_find_outlined,
                  title: 'Discover',
                  subtitle: 'Find devices nearby',
                  onTap: () => _navigateTo(context, const DiscoveryScreen()),
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.screen_share_outlined,
                  title: 'Screen mirror',
                  subtitle: 'Show the desktop screen',
                  enabled: hasDesktop,
                  onTap: hasDesktop
                      ? () {
                          final desktop = connectedDevices.firstWhere(
                            (d) => d.type == 'desktop',
                          );
                          _navigateTo(
                            context,
                            ScreenMirrorScreen(
                              deviceId: desktop.id,
                              deviceName: desktop.name,
                            ),
                          );
                        }
                      : null,
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.mouse_outlined,
                  title: 'Remote input',
                  subtitle: 'Trackpad mode',
                  enabled: hasDesktop,
                  onTap: hasDesktop
                      ? () {
                          final desktop = connectedDevices.firstWhere(
                            (d) => d.type == 'desktop',
                          );
                          _navigateTo(
                            context,
                            RemoteInputScreen(
                              deviceId: desktop.id,
                              deviceName: desktop.name,
                            ),
                          );
                        }
                      : null,
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.bolt_outlined,
                  title: 'Automation',
                  subtitle: 'Trigger rules',
                  onTap: () => _navigateTo(context, const AutomationRulesScreen()),
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.content_paste_outlined,
                  title: 'Clipboard',
                  subtitle: 'History',
                  onTap: () => _navigateTo(context, const ClipboardScreen()),
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.chat_bubble_outline,
                  title: 'Messages',
                  subtitle: 'SMS threads',
                  onTap: () => _navigateTo(context, const MessagesScreen()),
                ),
                _buildActionTile(
                  context,
                  colors: colors,
                  icon: Icons.phone_outlined,
                  title: 'Calls',
                  subtitle: 'Call controls',
                  onTap: () => _navigateTo(context, const CallsScreen()),
                ),
              ],
            ),
          ),
          const SizedBox(height: 12),
        ],
      ),
    );
  }

  void _navigateTo(BuildContext context, Widget screen) {
    Navigator.pop(context); // Close the bottom sheet first
    Navigator.push(
      context,
      MaterialPageRoute(builder: (_) => screen),
    );
  }

  Widget _buildActionTile(
    BuildContext context, {
    required AppColors colors,
    required IconData icon,
    required String title,
    required String subtitle,
    VoidCallback? onTap,
    bool enabled = true,
  }) {
    return GestureDetector(
      onTap: enabled ? onTap : null,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        decoration: BoxDecoration(
          color: colors.bg2,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: enabled
                ? colors.border
                : colors.border.withValues(alpha: 0.5),
          ),
        ),
        child: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: colors.bg0,
                borderRadius: BorderRadius.circular(8),
              ),
              child: Icon(
                icon,
                color: enabled ? colors.accent : colors.text3,
                size: 18,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                      color: enabled ? colors.text1 : colors.text3,
                    ),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    subtitle,
                    style: TextStyle(
                      fontSize: 11,
                      color: enabled ? colors.text2 : colors.text3,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
