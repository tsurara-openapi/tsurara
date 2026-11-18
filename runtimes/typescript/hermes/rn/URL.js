/**
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 *
 *  strict-local
 * @format
 */

// flowlint unsafe-getters-setters:off


import NativeBlobModule from './NativeBlobModule';

let BLOB_URL_PREFIX = null;

if (
  NativeBlobModule &&
  typeof NativeBlobModule.getConstants().BLOB_URI_SCHEME === 'string'
) {
  const constants = NativeBlobModule.getConstants();
  // $FlowFixMe[incompatible-type] asserted above
  // $FlowFixMe[unsafe-addition]
  BLOB_URL_PREFIX = constants.BLOB_URI_SCHEME + ':';
  if (typeof constants.BLOB_URI_HOST === 'string') {
    BLOB_URL_PREFIX += `//${constants.BLOB_URI_HOST}/`;
  }
}

/*
 * To allow Blobs be accessed via `content://` URIs,
 * you need to register `BlobProvider` as a ContentProvider in your app's `AndroidManifest.xml`:
 *
 * ```xml
 * <manifest>
 *   <application>
 *     <provider
 *       android:name="com.facebook.react.modules.blob.BlobProvider"
 *       android:authorities="@string/blob_provider_authority"
 *       android:exported="false"
 *     />
 *   </application>
 * </manifest>
 * ```
 * And then define the `blob_provider_authority` string in `res/values/strings.xml`.
 * Use a dotted name that's entirely unique to your app:
 *
 * ```xml
 * <resources>
 *   <string name="blob_provider_authority">your.app.package.blobs</string>
 * </resources>
 * ```
 */

export {URLSearchParams} from './URLSearchParams';

function validateBaseUrl(url) {
  // from this MIT-licensed gist: https://gist.github.com/dperini/729294
  return /^(?:(?:(?:https?|ftp):)?\/\/)(?:(?:[1-9]\d?|1\d\d|2[01]\d|22[0-3])(?:\.(?:1?\d{1,2}|2[0-4]\d|25[0-5])){2}(?:\.(?:[1-9]\d?|1\d\d|2[0-4]\d|25[0-4]))|(?:(?:[a-z0-9\u00a1-\uffff][a-z0-9\u00a1-\uffff_-]{0,62})?[a-z0-9\u00a1-\uffff]\.)*(?:[a-z\u00a1-\uffff]{2,}\.?))(?::\d{2,5})?(?:[/?#]\S*)?$/.test(
    url,
  );
}

export class URL {
  _url;
  _searchParamsInstance = null;

  static createObjectURL(blob) {
    if (BLOB_URL_PREFIX === null) {
      throw new Error('Cannot create URL for blob!');
    }
    return `${BLOB_URL_PREFIX}${blob.data.blobId}?offset=${blob.data.offset}&size=${blob.size}`;
  }

  static revokeObjectURL(url) {
    // Do nothing.
  }

  // $FlowFixMe[missing-local-annot]
  constructor(url, base) {
    let baseUrl = null;
    // $FlowFixMe[sketchy-null-string]
    if (!base || validateBaseUrl(url)) {
      this._url = url;
      if (this._url.includes('#')) {
        const split = this._url.split('#');
        const beforeHash = split[0];
        const website = beforeHash.split('://')[1];
        if (!website.includes('/')) {
          this._url = split.join('/#');
        }
      }

      // Only add trailing slash if URL has no path (just domain)
      if (
        !this._url.endsWith('/') &&
        !(this._url.includes('?') || this._url.includes('#'))
      ) {
        const afterProtocol = this._url.split('://')[1];
        if (afterProtocol && !afterProtocol.includes('/')) {
          this._url += '/';
        }
      }
    } else {
      if (typeof base === 'string') {
        baseUrl = base;
        if (!validateBaseUrl(baseUrl)) {
          throw new TypeError(`Invalid base URL: ${baseUrl}`);
        }
      } else {
        baseUrl = base.toString();
      }
      if (baseUrl.endsWith('/')) {
        baseUrl = baseUrl.slice(0, baseUrl.length - 1);
      }
      let path = url;
      if (!path.startsWith('/')) {
        path = `/${path}`;
      }
      if (baseUrl.endsWith(path)) {
        path = '';
      }
      this._url = `${baseUrl}${path}`;
    }
  }

  get hash() {
    const hashMatch = this._url.match(/#([^/]*)/);
    return hashMatch ? `#${hashMatch[1]}` : '';
  }

  get host() {
    const hostMatch = this._url.match(/^https?:\/\/(?:[^@]+@)?([^:/?#]+)/);
    const portMatch = this._url.match(/:(\d+)(?=[/?#]|$)/);
    return hostMatch
      ? hostMatch[1] + (portMatch ? `:${portMatch[1]}` : '')
      : '';
  }

  get hostname() {
    const hostnameMatch = this._url.match(/^https?:\/\/(?:[^@]+@)?([^:/?#]+)/);
    return hostnameMatch ? hostnameMatch[1] : '';
  }

  get href() {
    return this.toString();
  }

  get origin() {
    const matches = this._url.match(/^(https?:\/\/[^/]+)/);
    return matches ? matches[1] : '';
  }

  get password() {
    const passwordMatch = this._url.match(/https?:\/\/.*:(.*)@/);
    return passwordMatch ? passwordMatch[1] : '';
  }

  get pathname() {
    const pathMatch = this._url.match(/https?:\/\/[^/]+(\/[^?#]*)?/);
    return pathMatch ? pathMatch[1] || '/' : '/';
  }

  get port() {
    const portMatch = this._url.match(/:(\d+)(?=[/?#]|$)/);
    return portMatch ? portMatch[1] : '';
  }

  get protocol() {
    const protocolMatch = this._url.match(/^([a-zA-Z][a-zA-Z\d+\-.]*):/);
    return protocolMatch ? protocolMatch[1] + ':' : '';
  }

  get search() {
    const searchMatch = this._url.match(/\?([^#]*)/);
    return searchMatch ? `?${searchMatch[1]}` : '';
  }

  set search(value) {
    // Remove leading '?' if present
    const searchString = value.startsWith('?') ? value.slice(1) : value;

    // Update the internal URL
    const baseUrl = this._url.split('?')[0].split('#')[0];
    const hash = this.hash;

    if (searchString) {
      this._url = baseUrl + '?' + searchString + hash;
    } else {
      this._url = baseUrl + hash;
    }

    // Reset the searchParams instance so it gets recreated with new values
    this._searchParamsInstance = null;
  }

  get searchParams() {
    if (this._searchParamsInstance == null) {
      this._searchParamsInstance = new URLSearchParams(this.search);
    }
    return this._searchParamsInstance;
  }

  toJSON() {
    return this.toString();
  }

  toString() {
    if (this._searchParamsInstance === null) {
      return this._url;
    }

    // Remove existing search params and hash from the URL
    const baseUrl = this._url.split('?')[0].split('#')[0];
    const hash = this.hash;

    // $FlowFixMe[incompatible-use]
    const instanceString = this._searchParamsInstance.toString();

    if (instanceString) {
      return baseUrl + '?' + instanceString + hash;
    } else {
      return baseUrl + hash;
    }
  }

  get username() {
    const usernameMatch = this._url.match(/^https?:\/\/([^:@]+)(?::[^@]*)?@/);
    return usernameMatch ? usernameMatch[1] : '';
  }
}
