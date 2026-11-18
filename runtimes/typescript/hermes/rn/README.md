# React Native polyfills

`URL.js`, `URLSearchParams.js` and `FormData.js` are React Native 0.88.1's
(`node_modules/react-native/Libraries/Blob/` and `Libraries/Network/`, MIT, Copyright (c) Meta Platforms,
Inc. and affiliates), with the Flow types removed by `flow-remove-types --pretty`. `NativeBlobModule.js`
replaces the native module with `null`, as on a VM without React Native.

To update them, run `flow-remove-types --pretty` on the same files of a newer `react-native` package.
