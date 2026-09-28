import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/widgets/context_menu.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  testWidgets('renders child widget', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: ContextMenu(
            items: [
              ContextMenuItem(icon: Icons.copy, label: 'Copy', onTap: () {}),
            ],
            child: const Text('Long press me'),
          ),
        ),
      ),
    );

    expect(find.text('Long press me'), findsOneWidget);
  });

  testWidgets('shows context menu on long press', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: ContextMenu(
            items: [
              ContextMenuItem(icon: Icons.copy, label: 'Copy', onTap: () {}),
              ContextMenuItem(icon: Icons.delete, label: 'Delete', onTap: () {}),
            ],
            child: const Text('Long press me'),
          ),
        ),
      ),
    );

    await tester.longPress(find.text('Long press me'));
    await tester.pumpAndSettle();

    expect(find.text('Copy'), findsOneWidget);
    expect(find.text('Delete'), findsOneWidget);
  });
}
