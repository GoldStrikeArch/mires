/// workbox-precaching 7.4.1 with the workbox-core, workbox-routing and workbox-strategies parts it
/// uses (CONTRACT.md "API subset"), as one library: the packages are its parts, so a Strategy
/// subclass in precaching.dart overrides the library-private `_handle`, as Workbox's subclasses
/// override its protected one.
library;

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'host.dart';

part 'workbox_core.dart';
part 'strategies.dart';
part 'precaching.dart';
part 'routing.dart';
