import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/widgets/skeleton.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  group('Skeleton', () {
    testWidgets('renders with default dimensions', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.dark(),
          home: const Scaffold(body: Skeleton()),
        ),
      );

      expect(find.byType(Skeleton), findsOneWidget);
    });

    testWidgets('renders circle variant', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.dark(),
          home: const Scaffold(body: Skeleton.circle(size: 40)),
        ),
      );

      expect(find.byType(Skeleton), findsOneWidget);
    });
  });

  group('SkeletonList', () {
    testWidgets('renders correct number of skeleton items', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.dark(),
          home: const Scaffold(body: SkeletonList(itemCount: 3)),
        ),
      );

      expect(find.byType(Skeleton), findsNWidgets(3));
    });
  });
}
