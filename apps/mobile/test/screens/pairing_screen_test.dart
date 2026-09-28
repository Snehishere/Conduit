import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/pairing_screen.dart';
import 'package:conduit/services/pairing_service.dart';
import 'package:conduit/services/encryption_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/theme/app_theme.dart';

/// Test double that skips the permission_handler platform channel.
class FakePairingService extends PairingService {
  FakePairingService() : super(EncryptionService());

  @override
  Future<bool> requestCameraPermission() async => false;
}

Widget _app(Widget home) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<PairingService>(
        create: (_) => FakePairingService(),
      ),
      ChangeNotifierProvider<WebSocketService>(
        create: (_) => WebSocketService(),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: home,
    ),
  );
}

void main() {
  testWidgets('shows camera permission prompt when permission missing',
      (tester) async {
    await tester.pumpWidget(_app(const PairingScreen()));
    await tester.pump();

    expect(find.text('Pair device'), findsOneWidget);
    expect(find.text('Camera permission required'), findsOneWidget);
    expect(find.text('Grant permission'), findsOneWidget);
    expect(find.text('Enter code'), findsOneWidget);
  });

  testWidgets('toggles to manual pairing code entry', (tester) async {
    await tester.pumpWidget(_app(const PairingScreen()));
    await tester.pump();

    await tester.tap(find.text('Enter code'));
    await tester.pump();

    expect(find.text('Enter pairing code'), findsOneWidget);
    expect(find.byType(TextField), findsOneWidget);
    expect(
      find.byWidgetPredicate(
        (w) =>
            w is TextField &&
            w.decoration?.hintText == 'XXXX-XXXX-XXXX',
      ),
      findsOneWidget,
    );
    expect(find.text('Connect'), findsOneWidget);
    expect(find.text('Scan QR code'), findsOneWidget);
  });
}
