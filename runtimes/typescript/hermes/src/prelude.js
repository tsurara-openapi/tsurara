// The globals React Native installs before app code runs (Libraries/Core/setUpXHR.js and
// setUpTimers.js), as far as @tsurara/client uses them. The VM has none of them.
import 'whatwg-fetch';
// React Native 0.75 installs these from the same module (Libraries/Core/setUpXHR.js); 0.88 has its own.
import { AbortController, AbortSignal } from 'abort-controller/dist/abort-controller';
import FormData from '../rn/FormData.js';
import { URL, URLSearchParams } from '../rn/URL.js';

const g = globalThis;
g.FormData = FormData;
g.URL = URL;
g.URLSearchParams = URLSearchParams;
g.AbortController = AbortController;
g.AbortSignal = AbortSignal;
if (typeof g.queueMicrotask !== 'function') {
  g.queueMicrotask = (f) => Promise.resolve().then(f);
}
if (typeof g.console === 'undefined') {
  const log = (...args) => g.print(...args.map(String));
  g.console = { log, info: log, warn: log, error: log, debug: log, assert: () => {} };
}
