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

// Small subset from whatwg-url: https://github.com/jsdom/whatwg-url/tree/master/src
// The reference code bloat comes from Unicode issues with URLs, so those won't work here.
export class URLSearchParams {
  _searchParams = new Map();

  get size() {
    return this._searchParams.size;
  }

  constructor(params) {
    /* $FlowFixMe[invalid-compare] Error discovered during Constant Condition
     * roll out. See https://fburl.com/workplace/5whu3i34. */
    if (params === null) {
      return;
    }

    if (typeof params === 'string') {
      // URLSearchParams("key1=value1&key2=value2");
      params
        .replace(/^\?/, '')
        .split('&')
        .forEach(pair => {
          if (!pair) {
            return;
          }
          const [key, value] = pair
            .split('=')
            .map(part => decodeURIComponent(part.replace(/\+/g, ' ')));
          this.append(key, value);
        });
    } else if (Array.isArray(params)) {
      //URLSearchParams([["key1", "value1"], ["key2", "value2"]]);
      params.forEach(([key, value]) => this.append(key, value));
    } else if (typeof params === 'object') {
      //URLSearchParams({ key1: "value1", key2: "value2" });
      Object.entries(params).forEach(([key, value]) => this.append(key, value));
    }
  }

  append(key, value) {
    if (!this._searchParams.has(key)) {
      this._searchParams.set(key, [value]); // Initialize with an array if key is missing
    } else {
      this._searchParams.get(key)?.push(value); // Else push the value to the array
    }
  }

  delete(name) {
    this._searchParams.delete(name);
  }

  get(name) {
    const values = this._searchParams.get(name);
    return values ? values[0] : null;
  }

  getAll(name) {
    return this._searchParams.get(name) ?? [];
  }

  has(name) {
    return this._searchParams.has(name);
  }

  set(name, value) {
    this._searchParams.set(name, [value]);
  }

  keys() {
    return this._searchParams.keys();
  }

  values() {
    function* generateValues(params) {
      for (const valueArray of params.values()) {
        for (const value of valueArray) {
          yield value;
        }
      }
    }
    return generateValues(this._searchParams);
  }

  entries() {
    function* generateEntries(
      params,
    ) {
      for (const [key, values] of params) {
        for (const value of values) {
          yield [key, value];
        }
      }
    }

    return generateEntries(this._searchParams);
  }

  forEach(
    callback,
  ) {
    for (const [key, values] of this._searchParams) {
      for (const value of values) {
        callback(value, key, this);
      }
    }
  }

  sort() {
    this._searchParams = new Map(
      [...this._searchParams.entries()].sort(([a], [b]) => a.localeCompare(b)),
    );
  }

  // $FlowFixMe[unsupported-syntax]
  [Symbol.iterator]() {
    const entries = [];

    for (const [key, values] of this._searchParams) {
      for (const value of values) {
        entries.push([key, value]);
      }
    }

    // $FlowFixMe[incompatible-use]
    return entries[Symbol.iterator]();
  }

  toString() {
    return Array.from(this._searchParams.entries())
      .map(([key, values]) =>
        values
          .map(
            value =>
              `${encodeURIComponent(key).replace(/%20/g, '+')}=${encodeURIComponent(
                value,
              ).replace(/%20/g, '+')}`, // Convert only spaces to '+'
          )
          .join('&'),
      )
      .join('&');
  }
}
