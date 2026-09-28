import 'package:flutter/material.dart';
import 'theme/theme_provider.dart';
import 'screens/home_screen.dart';

class ConduitApp extends StatelessWidget {
  final ThemeProvider themeProvider;

  const ConduitApp({super.key, required this.themeProvider});

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: themeProvider,
      builder: (context, _) {
        return MaterialApp(
          title: 'Conduit',
          debugShowCheckedModeBanner: false,
          themeMode: themeProvider.themeMode,
          theme: themeProvider.lightTheme,
          darkTheme: themeProvider.darkTheme,
          home: const HomeScreen(),
        );
      },
    );
  }
}
