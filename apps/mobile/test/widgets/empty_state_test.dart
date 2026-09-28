import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/widgets/empty_state.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  testWidgets('renders title and description', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: const Scaffold(
          body: EmptyState(
            icon: Icons.folder_open,
            title: 'No files',
            description: 'Send a file to get started',
          ),
        ),
      ),
    );

    expect(find.text('No files'), findsOneWidget);
    expect(find.text('Send a file to get started'), findsOneWidget);
    expect(find.byIcon(Icons.folder_open), findsOneWidget);
  });

  testWidgets('renders action button when provided', (tester) async {
    var tapped = false;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: EmptyState(
            icon: Icons.inbox,
            title: 'Empty',
            description: 'Nothing here',
            actionLabel: 'Refresh',
            onAction: () => tapped = true,
          ),
        ),
      ),
    );

    expect(find.text('Refresh'), findsOneWidget);
    await tester.tap(find.text('Refresh'));
    expect(tapped, isTrue);
  });

  testWidgets('hides action button when not provided', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: const Scaffold(
          body: EmptyState(
            icon: Icons.inbox,
            title: 'Empty',
            description: 'Nothing here',
          ),
        ),
      ),
    );

    expect(find.byType(ElevatedButton), findsNothing);
  });
}
