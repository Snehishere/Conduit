import 'package:flutter/material.dart';
import 'package:permission_handler/permission_handler.dart';

class PermissionHelper {
  static Future<bool> requestNotificationPermission() async {
    final status = await Permission.notification.request();
    return status.isGranted;
  }

  static Future<bool> requestBluetoothPermission() async {
    final statuses = await [
      Permission.bluetooth,
      Permission.bluetoothScan,
      Permission.bluetoothConnect,
    ].request();
    return statuses.values.every((s) => s.isGranted);
  }

  static Future<bool> checkAllPermissions() async {
    final statuses = await [
      Permission.notification,
      Permission.bluetooth,
      Permission.bluetoothScan,
      Permission.bluetoothConnect,
    ].request();
    return statuses.values.every((s) => s.isGranted);
  }

  /// Request a permission with a user-friendly rationale dialog.
  /// Returns true if granted, shows a dialog explaining why and returns false if denied.
  static Future<bool> requestWithRationale(
    BuildContext context,
    Permission permission,
    String title,
    String rationale,
  ) async {
    if (await permission.isGranted) return true;

    final status = await permission.request();
    if (status.isGranted) return true;

    if (status.isPermanentlyDenied) {
      if (context.mounted) {
        showDialog(
          context: context,
          builder: (ctx) => AlertDialog(
            title: Text(title),
            content: Text('$rationale\n\nPlease enable it in App Settings.'),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(ctx).pop(),
                child: const Text('Cancel'),
              ),
              TextButton(
                onPressed: () {
                  openAppSettings();
                  Navigator.of(ctx).pop();
                },
                child: const Text('Open Settings'),
              ),
            ],
          ),
        );
      }
      return false;
    }

    return status.isGranted;
  }
}
