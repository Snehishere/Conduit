Pod::Spec.new do |s|
  s.name             = 'Flutter'
  s.version          = '1.0.0'
  s.summary          = 'Flutter iOS framework.'
s.description = 'Flutter iOS framework for Conduit.'
  s.homepage         = 'https://flutter.dev'
  s.license          = { :type => 'MIT' }
  s.author           = { 'Flutter' => 'flutter-dev@googlegroups.com' }
  s.source           = { :path => '.' }
  s.source_files     = '*.h'
  s.public_header_files = '*.h'
  s.platform         = :ios, '13.0'
  s.dependency 'Flutter'
end
