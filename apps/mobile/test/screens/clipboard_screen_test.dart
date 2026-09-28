import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/clipboard_screen.dart';
import 'package:conduit/services/clipboard_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(ClipboardService svc) {
  return ChangeNotifierProvider<ClipboardService>.value(
    value: svc,
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const ClipboardScreen(),
    ),
  );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('shows empty state when there is no clipboard history',
      (tester) async {
    await tester.pumpWidget(_app(ClipboardService()));
    await tester.pump();

    expect(find.text('Clipboard'), findsOneWidget);
    expect(find.text('Auto-sync paused'), findsOneWidget);
    expect(find.text('No clipboard history'), findsOneWidget);
    expect(
      find.text('Clipboard changes will appear here\nwhen auto-sync is active'),
      findsOneWidget,
    );
    expect(find.byTooltip('Clear history'), findsNothing);
  });

  testWidgets('renders a synced item and clears history via dialog',
      (tester) async {
    final svc = ClipboardService();
    svc.syncFromDevice('Hello from desktop', 'text/plain', 'desktop');

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    expect(find.text('Hello from desktop'), findsOneWidget);
    expect(find.textContaining('desktop ·'), findsOneWidget);
    expect(find.text('1 item'), findsOneWidget);
    expect(find.byTooltip('Clear history'), findsOneWidget);

    await tester.tap(find.byTooltip('Clear history'));
    await tester.pump();

    expect(find.text('Clear clipboard history?'), findsOneWidget);
    expect(find.text('This removes 1 item from history.'), findsOneWidget);

    await tester.tap(find.text('Clear'));
    await tester.pump();

    expect(find.text('Clear clipboard history?'), findsNothing);
    expect(find.text('No clipboard history'), findsOneWidget);
    expect(find.text('Hello from desktop'), findsNothing);
    expect(find.byTooltip('Clear history'), findsNothing);
  });
}
