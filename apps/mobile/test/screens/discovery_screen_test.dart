import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/discovery_screen.dart';
import 'package:conduit/services/discovery_service.dart';
import 'package:conduit/theme/app_theme.dart';

/// Test double that skips real mDNS socket binding (flaky/unavailable in
/// the test environment). Seeded devices are never cleared.
class FakeDiscoveryService extends DiscoveryService {
  @override
  Future<void> startDiscovery() async {}

  @override
  void stopDiscovery() {}
}

Widget _app(DiscoveryService svc) {
  return ChangeNotifierProvider<DiscoveryService>.value(
    value: svc,
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const DiscoveryScreen(),
    ),
  );
}

DiscoveredDevice _device({
  String id = 'd1',
  String name = 'Office PC',
  String type = 'desktop',
  String address = '192.168.1.5',
  int port = 9527,
}) {
  return DiscoveredDevice(
    id: id,
    name: name,
    type: type,
    address: address,
    port: port,
  );
}

void main() {
  // DiscoveryScreen shows an indeterminate CircularProgressIndicator —
  // use pump() instead of pumpAndSettle to avoid timing out.
  testWidgets('shows discovery guidance when no devices found',
      (tester) async {
    await tester.pumpWidget(_app(FakeDiscoveryService()));
    await tester.pump();

    expect(find.text('Discover devices'), findsOneWidget);
    expect(find.text('Tap the play button to start discovery'), findsOneWidget);
    expect(
      find.text('Make sure devices are on the same network'),
      findsOneWidget,
    );
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
  });

  testWidgets('renders a discovered device and shows its details dialog',
      (tester) async {
    final svc = FakeDiscoveryService();
    svc.deviceFound(_device());

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    expect(find.text('Office PC'), findsOneWidget);
    expect(find.text('desktop · 192.168.1.5'), findsOneWidget);

    await tester.tap(find.text('Office PC'));
    await tester.pump();

    expect(find.text('Office PC'), findsNWidgets(2));
    expect(find.text('Type: desktop'), findsOneWidget);
    expect(find.text('Address: 192.168.1.5'), findsOneWidget);
    expect(find.text('Service port: 9527'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
    expect(find.text('Pair'), findsOneWidget);
  });
}
