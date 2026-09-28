import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/files_screen.dart';
import 'package:conduit/services/file_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(FileService fileService) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<FileService>.value(value: fileService),
      ChangeNotifierProvider<WebSocketService>(
        create: (_) => WebSocketService(),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const FilesScreen(),
    ),
  );
}

/// Seed a pending incoming transfer without touching any platform channels.
FileService _seededService() {
  final svc = FileService();
  svc.handleFileRequest(<String, dynamic>{
    'id': 't1',
    'name': 'report.pdf',
    'size': 20480,
    'mime': 'application/pdf',
    'from': 'desktop-1',
  });
  return svc;
}

void main() {
  testWidgets('shows empty state when there are no transfers',
      (tester) async {
    await tester.pumpWidget(_app(FileService()));
    await tester.pump();

    expect(find.text('Files'), findsOneWidget);
    expect(find.text('No file transfers yet'), findsOneWidget);
    expect(
      find.text('Tap the send button to send a file to your desktop'),
      findsOneWidget,
    );
    expect(find.byTooltip('Send file'), findsOneWidget);
  });

  testWidgets('renders a pending transfer and accepts it', (tester) async {
    await tester.pumpWidget(_app(_seededService()));
    await tester.pump();

    expect(find.text('report.pdf'), findsOneWidget);
    expect(find.text('20.0 KB • Waiting to accept'), findsOneWidget);
    expect(find.byTooltip('Accept'), findsOneWidget);
    expect(find.byTooltip('Decline'), findsOneWidget);

    await tester.tap(find.byTooltip('Accept'));
    await tester.pump();

    expect(find.textContaining('Transferring'), findsOneWidget);
    expect(find.byType(LinearProgressIndicator), findsOneWidget);
    expect(find.byTooltip('Cancel'), findsOneWidget);
    expect(find.byTooltip('Accept'), findsNothing);
  });

  testWidgets('declines a pending transfer and marks it cancelled',
      (tester) async {
    await tester.pumpWidget(_app(_seededService()));
    await tester.pump();

    await tester.tap(find.byTooltip('Decline'));
    await tester.pump();

    expect(find.text('20.0 KB • Cancelled'), findsOneWidget);
    expect(find.byTooltip('Accept'), findsNothing);
    expect(find.byTooltip('Decline'), findsNothing);
  });
}
