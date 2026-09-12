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
}
