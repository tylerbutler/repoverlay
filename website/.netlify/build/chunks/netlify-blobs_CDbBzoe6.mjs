import 'process';
import Bt from 'node:http';
import zs from 'node:https';
import st from 'node:zlib';
import me, { PassThrough, pipeline } from 'node:stream';
import { Buffer as Buffer$1 } from 'node:buffer';
import { promisify, deprecate, types } from 'node:util';
import { format } from 'node:url';
import { isIP } from 'node:net';
import { promises, statSync, createReadStream } from 'node:fs';
import { basename } from 'node:path';

const suspectProtoRx = /"(?:_|\\u0{2}5[Ff]){2}(?:p|\\u0{2}70)(?:r|\\u0{2}72)(?:o|\\u0{2}6[Ff])(?:t|\\u0{2}74)(?:o|\\u0{2}6[Ff])(?:_|\\u0{2}5[Ff]){2}"\s*:/;
const suspectConstructorRx = /"(?:c|\\u0063)(?:o|\\u006[Ff])(?:n|\\u006[Ee])(?:s|\\u0073)(?:t|\\u0074)(?:r|\\u0072)(?:u|\\u0075)(?:c|\\u0063)(?:t|\\u0074)(?:o|\\u006[Ff])(?:r|\\u0072)"\s*:/;
const JsonSigRx = /^\s*["[{]|^\s*-?\d{1,16}(\.\d{1,17})?([Ee][+-]?\d+)?\s*$/;
function jsonParseTransform(key, value) {
  if (key === "__proto__" || key === "constructor" && value && typeof value === "object" && "prototype" in value) {
    warnKeyDropped(key);
    return;
  }
  return value;
}
function warnKeyDropped(key) {
  console.warn(`[destr] Dropping "${key}" key to prevent prototype pollution.`);
}
function destr(value, options = {}) {
  if (typeof value !== "string") {
    return value;
  }
  if (value[0] === '"' && value[value.length - 1] === '"' && value.indexOf("\\") === -1) {
    return value.slice(1, -1);
  }
  const _value = value.trim();
  if (_value.length <= 9) {
    switch (_value.toLowerCase()) {
      case "true": {
        return true;
      }
      case "false": {
        return false;
      }
      case "undefined": {
        return void 0;
      }
      case "null": {
        return null;
      }
      case "nan": {
        return Number.NaN;
      }
      case "infinity": {
        return Number.POSITIVE_INFINITY;
      }
      case "-infinity": {
        return Number.NEGATIVE_INFINITY;
      }
    }
  }
  if (!JsonSigRx.test(value)) {
    if (options.strict) {
      throw new SyntaxError("[destr] Invalid JSON");
    }
    return value;
  }
  try {
    if (suspectProtoRx.test(value) || suspectConstructorRx.test(value)) {
      if (options.strict) {
        throw new Error("[destr] Possible prototype pollution");
      }
      return JSON.parse(value, jsonParseTransform);
    }
    return JSON.parse(value);
  } catch (error) {
    if (options.strict) {
      throw error;
    }
    return value;
  }
}

function defineDriver(factory) {
  return factory;
}
function createError(driver, message, opts) {
  const err = new Error(`[unstorage] [${driver}] ${message}`, opts);
  if (Error.captureStackTrace) {
    Error.captureStackTrace(err, createError);
  }
  return err;
}
function createRequiredError(driver, name) {
  if (Array.isArray(name)) {
    return createError(
      driver,
      `Missing some of the required options ${name.map((n) => "`" + n + "`").join(", ")}`
    );
  }
  return createError(driver, `Missing required option \`${name}\`.`);
}

// src/lib/base64.ts
var getString = (input) => typeof input === "string" ? input : JSON.stringify(input);
var base64Decode = globalThis.Buffer ? (input) => Buffer.from(input, "base64").toString() : (input) => atob(input);
var base64Encode = globalThis.Buffer ? (input) => Buffer.from(getString(input)).toString("base64") : (input) => btoa(getString(input));

// src/lib/environment.ts
var getEnvironment = () => {
  const { Deno, Netlify, process } = globalThis;
  return Netlify?.env ?? Deno?.env ?? {
    delete: (key) => delete process?.env[key],
    get: (key) => process?.env[key],
    has: (key) => Boolean(process?.env[key]),
    set: (key, value) => {
      if (process?.env) {
        process.env[key] = value;
      }
    },
    toObject: () => process?.env ?? {}
  };
};

// src/constants.ts
var GET_TRACER = "__netlify__getTracer";

// src/main.ts
var getTracer = (name, version) => {
  return globalThis[GET_TRACER]?.(name, version);
};
function withActiveSpan(tracer, name, optionsOrFn, contextOrFn, fn) {
  const func = typeof optionsOrFn === "function" ? optionsOrFn : fn;
  if (!func) {
    throw new Error("function to execute with active span is missing");
  }
  if (!tracer) {
    return func();
  }
  return tracer.withActiveSpan(name, optionsOrFn, contextOrFn, func);
}

// src/environment.ts
var getEnvironmentContext = () => {
  const context = globalThis.netlifyBlobsContext || getEnvironment().get("NETLIFY_BLOBS_CONTEXT");
  if (typeof context !== "string" || !context) {
    return {};
  }
  const data = base64Decode(context);
  try {
    return JSON.parse(data);
  } catch {
  }
  return {};
};
var MissingBlobsEnvironmentError = class extends Error {
  constructor(requiredProperties) {
    super(
      `The environment has not been configured to use Netlify Blobs. To use it manually, supply the following properties when creating a store: ${requiredProperties.join(
        ", "
      )}`
    );
    this.name = "MissingBlobsEnvironmentError";
  }
};
var BASE64_PREFIX = "b64;";
var METADATA_HEADER_INTERNAL = "x-amz-meta-user";
var METADATA_HEADER_EXTERNAL = "netlify-blobs-metadata";
var METADATA_MAX_SIZE = 2 * 1024;
var encodeMetadata = (metadata) => {
  if (!metadata) {
    return null;
  }
  const encodedObject = base64Encode(JSON.stringify(metadata));
  const payload = `b64;${encodedObject}`;
  if (METADATA_HEADER_EXTERNAL.length + payload.length > METADATA_MAX_SIZE) {
    throw new Error("Metadata object exceeds the maximum size");
  }
  return payload;
};
var decodeMetadata = (header) => {
  if (!header?.startsWith(BASE64_PREFIX)) {
    return {};
  }
  const encodedData = header.slice(BASE64_PREFIX.length);
  const decodedData = base64Decode(encodedData);
  const metadata = JSON.parse(decodedData);
  return metadata;
};
var getMetadataFromResponse = (response) => {
  if (!response.headers) {
    return {};
  }
  const value = response.headers.get(METADATA_HEADER_EXTERNAL) || response.headers.get(METADATA_HEADER_INTERNAL);
  try {
    return decodeMetadata(value);
  } catch {
    throw new Error(
      "An internal error occurred while trying to retrieve the metadata for an entry. Please try updating to the latest version of the Netlify Blobs client."
    );
  }
};

// src/headers.ts
var NF_ERROR = "x-nf-error";
var NF_REQUEST_ID = "x-nf-request-id";

// src/util.ts
var BlobsInternalError = class extends Error {
  constructor(res) {
    let details = res.headers.get(NF_ERROR) || `${res.status} status code`;
    if (res.headers.has(NF_REQUEST_ID)) {
      details += `, ID: ${res.headers.get(NF_REQUEST_ID)}`;
    }
    super(`Netlify Blobs has generated an internal error (${details})`);
    this.name = "BlobsInternalError";
  }
};
var collectIterator = async (iterator) => {
  const result = [];
  for await (const item of iterator) {
    result.push(item);
  }
  return result;
};
function withSpan(span, name, fn) {
  if (span) return fn(span);
  return withActiveSpan(getTracer(), name, (span2) => {
    return fn(span2);
  });
}

// src/consistency.ts
var BlobsConsistencyError = class extends Error {
  constructor() {
    super(
      `Netlify Blobs has failed to perform a read using strong consistency because the environment has not been configured with a 'uncachedEdgeURL' property`
    );
    this.name = "BlobsConsistencyError";
  }
};

// src/region.ts
var REGION_AUTO = "auto";
var regions = {
  "us-east-1": true,
  "us-east-2": true,
  "eu-central-1": true,
  "ap-southeast-1": true,
  "ap-southeast-2": true
};
var isValidRegion = (input) => Object.keys(regions).includes(input);
var InvalidBlobsRegionError = class extends Error {
  constructor(region) {
    super(
      `${region} is not a supported Netlify Blobs region. Supported values are: ${Object.keys(regions).join(", ")}.`
    );
    this.name = "InvalidBlobsRegionError";
  }
};
var DEFAULT_RETRY_DELAY = getEnvironment().get("NODE_ENV") === "test" ? 1 : 5e3;
var MIN_RETRY_DELAY = 1e3;
var MAX_RETRY = 5;
var RATE_LIMIT_HEADER = "X-RateLimit-Reset";
var fetchAndRetry = async (fetch, url, options, attemptsLeft = MAX_RETRY) => {
  try {
    const res = await fetch(url, options);
    if (attemptsLeft > 0 && (res.status === 429 || res.status >= 500)) {
      const delay = getDelay(res.headers.get(RATE_LIMIT_HEADER));
      await sleep(delay);
      return fetchAndRetry(fetch, url, options, attemptsLeft - 1);
    }
    return res;
  } catch (error) {
    if (attemptsLeft === 0) {
      throw error;
    }
    const delay = getDelay();
    await sleep(delay);
    return fetchAndRetry(fetch, url, options, attemptsLeft - 1);
  }
};
var getDelay = (rateLimitReset) => {
  if (!rateLimitReset) {
    return DEFAULT_RETRY_DELAY;
  }
  return Math.max(Number(rateLimitReset) * 1e3 - Date.now(), MIN_RETRY_DELAY);
};
var sleep = (ms) => new Promise((resolve) => {
  setTimeout(resolve, ms);
});

// src/client.ts
var SIGNED_URL_ACCEPT_HEADER = "application/json;type=signed-url";
var Client = class {
  constructor({ apiURL, consistency, edgeURL, fetch, region, siteID, token, uncachedEdgeURL }) {
    this.apiURL = apiURL;
    this.consistency = consistency ?? "eventual";
    this.edgeURL = edgeURL;
    this.fetch = fetch ?? globalThis.fetch;
    this.region = region;
    this.siteID = siteID;
    this.token = token;
    this.uncachedEdgeURL = uncachedEdgeURL;
    if (!this.fetch) {
      throw new Error(
        "Netlify Blobs could not find a `fetch` client in the global scope. You can either update your runtime to a version that includes `fetch` (like Node.js 18.0.0 or above), or you can supply your own implementation using the `fetch` property."
      );
    }
  }
  async getFinalRequest({
    consistency: opConsistency,
    key,
    metadata,
    method,
    parameters = {},
    storeName
  }) {
    const encodedMetadata = encodeMetadata(metadata);
    const consistency = opConsistency ?? this.consistency;
    let urlPath = `/${this.siteID}`;
    if (storeName) {
      urlPath += `/${storeName}`;
    }
    if (key) {
      urlPath += `/${key}`;
    }
    if (this.edgeURL) {
      if (consistency === "strong" && !this.uncachedEdgeURL) {
        throw new BlobsConsistencyError();
      }
      const headers = {
        authorization: `Bearer ${this.token}`
      };
      if (encodedMetadata) {
        headers[METADATA_HEADER_INTERNAL] = encodedMetadata;
      }
      if (this.region) {
        urlPath = `/region:${this.region}${urlPath}`;
      }
      const url2 = new URL(urlPath, consistency === "strong" ? this.uncachedEdgeURL : this.edgeURL);
      for (const key2 in parameters) {
        url2.searchParams.set(key2, parameters[key2]);
      }
      return {
        headers,
        url: url2.toString()
      };
    }
    const apiHeaders = { authorization: `Bearer ${this.token}` };
    const url = new URL(`/api/v1/blobs${urlPath}`, this.apiURL ?? "https://api.netlify.com");
    for (const key2 in parameters) {
      url.searchParams.set(key2, parameters[key2]);
    }
    if (this.region) {
      url.searchParams.set("region", this.region);
    }
    if (storeName === void 0 || key === void 0) {
      return {
        headers: apiHeaders,
        url: url.toString()
      };
    }
    if (encodedMetadata) {
      apiHeaders[METADATA_HEADER_EXTERNAL] = encodedMetadata;
    }
    if (method === "head" /* HEAD */ || method === "delete" /* DELETE */) {
      return {
        headers: apiHeaders,
        url: url.toString()
      };
    }
    const res = await this.fetch(url.toString(), {
      headers: { ...apiHeaders, accept: SIGNED_URL_ACCEPT_HEADER },
      method
    });
    if (res.status !== 200) {
      throw new BlobsInternalError(res);
    }
    const { url: signedURL } = await res.json();
    const userHeaders = encodedMetadata ? { [METADATA_HEADER_INTERNAL]: encodedMetadata } : void 0;
    return {
      headers: userHeaders,
      url: signedURL
    };
  }
  async makeRequest({
    body,
    conditions = {},
    consistency,
    headers: extraHeaders,
    key,
    metadata,
    method,
    parameters,
    storeName
  }) {
    const { headers: baseHeaders = {}, url } = await this.getFinalRequest({
      consistency,
      key,
      metadata,
      method,
      parameters,
      storeName
    });
    const headers = {
      ...baseHeaders,
      ...extraHeaders
    };
    if (method === "put" /* PUT */) {
      headers["cache-control"] = "max-age=0, stale-while-revalidate=60";
    }
    if ("onlyIfMatch" in conditions && conditions.onlyIfMatch) {
      headers["if-match"] = conditions.onlyIfMatch;
    } else if ("onlyIfNew" in conditions && conditions.onlyIfNew) {
      headers["if-none-match"] = "*";
    }
    const options = {
      body,
      headers,
      method
    };
    if (body instanceof ReadableStream) {
      options.duplex = "half";
    }
    return fetchAndRetry(this.fetch, url, options);
  }
};
var getClientOptions = (options, contextOverride) => {
  const context = contextOverride ?? getEnvironmentContext();
  const siteID = context.siteID ?? options.siteID;
  const token = context.token ?? options.token;
  if (!siteID || !token) {
    throw new MissingBlobsEnvironmentError(["siteID", "token"]);
  }
  if (options.region !== void 0 && !isValidRegion(options.region)) {
    throw new InvalidBlobsRegionError(options.region);
  }
  const clientOptions = {
    apiURL: context.apiURL ?? options.apiURL,
    consistency: options.consistency,
    edgeURL: context.edgeURL ?? options.edgeURL,
    fetch: options.fetch,
    region: options.region,
    siteID,
    token,
    uncachedEdgeURL: context.uncachedEdgeURL ?? options.uncachedEdgeURL
  };
  return clientOptions;
};

// src/store.ts
var DEPLOY_STORE_PREFIX = "deploy:";
var LEGACY_STORE_INTERNAL_PREFIX = "netlify-internal/legacy-namespace/";
var SITE_STORE_PREFIX = "site:";
var STATUS_OK = 200;
var STATUS_PRE_CONDITION_FAILED = 412;
var Store = class _Store {
  constructor(options) {
    this.client = options.client;
    if ("deployID" in options) {
      _Store.validateDeployID(options.deployID);
      let name = DEPLOY_STORE_PREFIX + options.deployID;
      if (options.name) {
        name += `:${options.name}`;
      }
      this.name = name;
    } else if (options.name.startsWith(LEGACY_STORE_INTERNAL_PREFIX)) {
      const storeName = options.name.slice(LEGACY_STORE_INTERNAL_PREFIX.length);
      _Store.validateStoreName(storeName);
      this.name = storeName;
    } else {
      _Store.validateStoreName(options.name);
      this.name = SITE_STORE_PREFIX + options.name;
    }
  }
  async delete(key) {
    const res = await this.client.makeRequest({ key, method: "delete" /* DELETE */, storeName: this.name });
    if (![200, 204, 404].includes(res.status)) {
      throw new BlobsInternalError(res);
    }
  }
  async deleteAll() {
    let totalDeletedBlobs = 0;
    let hasMore = true;
    while (hasMore) {
      const res = await this.client.makeRequest({ method: "delete" /* DELETE */, storeName: this.name });
      if (res.status !== 200) {
        throw new BlobsInternalError(res);
      }
      const data = await res.json();
      if (typeof data.blobs_deleted !== "number") {
        throw new BlobsInternalError(res);
      }
      totalDeletedBlobs += data.blobs_deleted;
      hasMore = typeof data.has_more === "boolean" && data.has_more;
    }
    return {
      deletedBlobs: totalDeletedBlobs
    };
  }
  async get(key, options) {
    return withSpan(options?.span, "blobs.get", async (span) => {
      const { consistency, type } = options ?? {};
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.key": key,
        "blobs.type": type,
        "blobs.method": "GET",
        "blobs.consistency": consistency
      });
      const res = await this.client.makeRequest({
        consistency,
        key,
        method: "get" /* GET */,
        storeName: this.name
      });
      span?.setAttributes({
        "blobs.response.body.size": res.headers.get("content-length") ?? void 0,
        "blobs.response.status": res.status
      });
      if (res.status === 404) {
        return null;
      }
      if (res.status !== 200) {
        throw new BlobsInternalError(res);
      }
      if (type === void 0 || type === "text") {
        return res.text();
      }
      if (type === "arrayBuffer") {
        return res.arrayBuffer();
      }
      if (type === "blob") {
        return res.blob();
      }
      if (type === "json") {
        return res.json();
      }
      if (type === "stream") {
        return res.body;
      }
      throw new BlobsInternalError(res);
    });
  }
  async getMetadata(key, options = {}) {
    return withSpan(options?.span, "blobs.getMetadata", async (span) => {
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.key": key,
        "blobs.method": "HEAD",
        "blobs.consistency": options.consistency
      });
      const res = await this.client.makeRequest({
        consistency: options.consistency,
        key,
        method: "head" /* HEAD */,
        storeName: this.name
      });
      span?.setAttributes({
        "blobs.response.status": res.status
      });
      if (res.status === 404) {
        return null;
      }
      if (res.status !== 200 && res.status !== 304) {
        throw new BlobsInternalError(res);
      }
      const etag = res?.headers.get("etag") ?? void 0;
      const metadata = getMetadataFromResponse(res);
      const result = {
        etag,
        metadata
      };
      return result;
    });
  }
  async getWithMetadata(key, options) {
    return withSpan(options?.span, "blobs.getWithMetadata", async (span) => {
      const { consistency, etag: requestETag, type } = options ?? {};
      const headers = requestETag ? { "if-none-match": requestETag } : void 0;
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.key": key,
        "blobs.method": "GET",
        "blobs.consistency": options?.consistency,
        "blobs.type": type,
        "blobs.request.etag": requestETag
      });
      const res = await this.client.makeRequest({
        consistency,
        headers,
        key,
        method: "get" /* GET */,
        storeName: this.name
      });
      const responseETag = res?.headers.get("etag") ?? void 0;
      span?.setAttributes({
        "blobs.response.body.size": res.headers.get("content-length") ?? void 0,
        "blobs.response.etag": responseETag,
        "blobs.response.status": res.status
      });
      if (res.status === 404) {
        return null;
      }
      if (res.status !== 200 && res.status !== 304) {
        throw new BlobsInternalError(res);
      }
      const metadata = getMetadataFromResponse(res);
      const result = {
        etag: responseETag,
        metadata
      };
      if (res.status === 304 && requestETag) {
        return { data: null, ...result };
      }
      if (type === void 0 || type === "text") {
        return { data: await res.text(), ...result };
      }
      if (type === "arrayBuffer") {
        return { data: await res.arrayBuffer(), ...result };
      }
      if (type === "blob") {
        return { data: await res.blob(), ...result };
      }
      if (type === "json") {
        return { data: await res.json(), ...result };
      }
      if (type === "stream") {
        return { data: res.body, ...result };
      }
      throw new Error(`Invalid 'type' property: ${type}. Expected: arrayBuffer, blob, json, stream, or text.`);
    });
  }
  list(options = {}) {
    return withSpan(options.span, "blobs.list", (span) => {
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.method": "GET",
        "blobs.list.paginate": options.paginate ?? false
      });
      const iterator = this.getListIterator(options);
      if (options.paginate) {
        return iterator;
      }
      return collectIterator(iterator).then(
        (items) => items.reduce(
          (acc, item) => ({
            blobs: [...acc.blobs, ...item.blobs],
            directories: [...acc.directories, ...item.directories]
          }),
          { blobs: [], directories: [] }
        )
      );
    });
  }
  async set(key, data, options = {}) {
    return withSpan(options.span, "blobs.set", async (span) => {
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.key": key,
        "blobs.method": "PUT",
        "blobs.data.size": typeof data == "string" ? data.length : data instanceof Blob ? data.size : data.byteLength,
        "blobs.data.type": typeof data == "string" ? "string" : data instanceof Blob ? "blob" : "arrayBuffer",
        "blobs.atomic": Boolean(options.onlyIfMatch ?? options.onlyIfNew)
      });
      _Store.validateKey(key);
      const conditions = _Store.getConditions(options);
      const res = await this.client.makeRequest({
        conditions,
        body: data,
        key,
        metadata: options.metadata,
        method: "put" /* PUT */,
        storeName: this.name
      });
      const etag = res.headers.get("etag") ?? "";
      span?.setAttributes({
        "blobs.response.etag": etag,
        "blobs.response.status": res.status
      });
      if (conditions) {
        return res.status === STATUS_PRE_CONDITION_FAILED ? { modified: false } : { etag, modified: true };
      }
      if (res.status === STATUS_OK) {
        return {
          etag,
          modified: true
        };
      }
      throw new BlobsInternalError(res);
    });
  }
  async setJSON(key, data, options = {}) {
    return withSpan(options.span, "blobs.setJSON", async (span) => {
      span?.setAttributes({
        "blobs.store": this.name,
        "blobs.key": key,
        "blobs.method": "PUT",
        "blobs.data.type": "json"
      });
      _Store.validateKey(key);
      const conditions = _Store.getConditions(options);
      const payload = JSON.stringify(data);
      const headers = {
        "content-type": "application/json"
      };
      const res = await this.client.makeRequest({
        ...conditions,
        body: payload,
        headers,
        key,
        metadata: options.metadata,
        method: "put" /* PUT */,
        storeName: this.name
      });
      const etag = res.headers.get("etag") ?? "";
      span?.setAttributes({
        "blobs.response.etag": etag,
        "blobs.response.status": res.status
      });
      if (conditions) {
        return res.status === STATUS_PRE_CONDITION_FAILED ? { modified: false } : { etag, modified: true };
      }
      if (res.status === STATUS_OK) {
        return {
          etag,
          modified: true
        };
      }
      throw new BlobsInternalError(res);
    });
  }
  static formatListResultBlob(result) {
    if (!result.key) {
      return null;
    }
    return {
      etag: result.etag,
      key: result.key
    };
  }
  static getConditions(options) {
    if ("onlyIfMatch" in options && "onlyIfNew" in options) {
      throw new Error(
        `The 'onlyIfMatch' and 'onlyIfNew' options are mutually exclusive. Using 'onlyIfMatch' will make the write succeed only if there is an entry for the key with the given content, while 'onlyIfNew' will make the write succeed only if there is no entry for the key.`
      );
    }
    if ("onlyIfMatch" in options && options.onlyIfMatch) {
      if (typeof options.onlyIfMatch !== "string") {
        throw new Error(`The 'onlyIfMatch' property expects a string representing an ETag.`);
      }
      return {
        onlyIfMatch: options.onlyIfMatch
      };
    }
    if ("onlyIfNew" in options && options.onlyIfNew) {
      if (typeof options.onlyIfNew !== "boolean") {
        throw new Error(
          `The 'onlyIfNew' property expects a boolean indicating whether the write should fail if an entry for the key already exists.`
        );
      }
      return {
        onlyIfNew: true
      };
    }
  }
  static validateKey(key) {
    if (key === "") {
      throw new Error("Blob key must not be empty.");
    }
    if (key.startsWith("/") || key.startsWith("%2F")) {
      throw new Error("Blob key must not start with forward slash (/).");
    }
    if (new TextEncoder().encode(key).length > 600) {
      throw new Error(
        "Blob key must be a sequence of Unicode characters whose UTF-8 encoding is at most 600 bytes long."
      );
    }
  }
  static validateDeployID(deployID) {
    if (!/^\w{1,24}$/.test(deployID)) {
      throw new Error(`'${deployID}' is not a valid Netlify deploy ID.`);
    }
  }
  static validateStoreName(name) {
    if (name.includes("/") || name.includes("%2F")) {
      throw new Error("Store name must not contain forward slashes (/).");
    }
    if (new TextEncoder().encode(name).length > 64) {
      throw new Error(
        "Store name must be a sequence of Unicode characters whose UTF-8 encoding is at most 64 bytes long."
      );
    }
  }
  getListIterator(options) {
    const { client, name: storeName } = this;
    const parameters = {};
    if (options?.prefix) {
      parameters.prefix = options.prefix;
    }
    if (options?.directories) {
      parameters.directories = "true";
    }
    return {
      [Symbol.asyncIterator]() {
        let currentCursor = null;
        let done = false;
        return {
          async next() {
            return withSpan(options?.span, "blobs.list.next", async (span) => {
              span?.setAttributes({
                "blobs.store": storeName,
                "blobs.method": "GET",
                "blobs.list.paginate": options?.paginate ?? false,
                "blobs.list.done": done,
                "blobs.list.cursor": currentCursor ?? void 0
              });
              if (done) {
                return { done: true, value: void 0 };
              }
              const nextParameters = { ...parameters };
              if (currentCursor !== null) {
                nextParameters.cursor = currentCursor;
              }
              const res = await client.makeRequest({
                method: "get" /* GET */,
                parameters: nextParameters,
                storeName
              });
              span?.setAttributes({
                "blobs.response.status": res.status
              });
              let blobs = [];
              let directories = [];
              if (![200, 204, 404].includes(res.status)) {
                throw new BlobsInternalError(res);
              }
              if (res.status === 404) {
                done = true;
              } else {
                const page = await res.json();
                if (page.next_cursor) {
                  currentCursor = page.next_cursor;
                } else {
                  done = true;
                }
                blobs = (page.blobs ?? []).map(_Store.formatListResultBlob).filter(Boolean);
                directories = page.directories ?? [];
              }
              return {
                done: false,
                value: {
                  blobs,
                  directories
                }
              };
            });
          }
        };
      }
    };
  }
};

// src/store_factory.ts
var getDeployStore = (input = {}, options) => {
  const context = getEnvironmentContext();
  const mergedOptions = typeof input === "string" ? { ...options, name: input } : input;
  const deployID = mergedOptions.deployID ?? context.deployID;
  if (!deployID) {
    throw new MissingBlobsEnvironmentError(["deployID"]);
  }
  const clientOptions = getClientOptions(mergedOptions, context);
  if (!clientOptions.region) {
    if (clientOptions.edgeURL || clientOptions.uncachedEdgeURL) {
      if (!context.primaryRegion) {
        throw new Error(
          "When accessing a deploy store, the Netlify Blobs client needs to be configured with a region, and one was not found in the environment. To manually set the region, set the `region` property in the `getDeployStore` options. If you are using the Netlify CLI, you may have an outdated version; run `npm install -g netlify-cli@latest` to update and try again."
        );
      }
      clientOptions.region = context.primaryRegion;
    } else {
      clientOptions.region = REGION_AUTO;
    }
  }
  const client = new Client(clientOptions);
  return new Store({ client, deployID, name: mergedOptions.name });
};
var getStore = (input, options) => {
  if (typeof input === "string") {
    const contextOverride = void 0;
    const clientOptions = getClientOptions({}, contextOverride);
    const client = new Client(clientOptions);
    return new Store({ client, name: input });
  }
  if (typeof input?.name === "string") {
    const { name } = input;
    const contextOverride = input?.siteID && input?.token ? { siteID: input?.siteID, token: input?.token } : void 0;
    const clientOptions = getClientOptions(input, contextOverride);
    if (!name) {
      throw new MissingBlobsEnvironmentError(["name"]);
    }
    const client = new Client(clientOptions);
    return new Store({ client, name });
  }
  if (typeof input?.deployID === "string") {
    const clientOptions = getClientOptions(input);
    const { deployID } = input;
    if (!deployID) {
      throw new MissingBlobsEnvironmentError(["deployID"]);
    }
    const client = new Client(clientOptions);
    return new Store({ client, deployID });
  }
  throw new Error(
    "The `getStore` method requires the name of the store as a string or as the `name` property of an options object"
  );
};

var t=Object.defineProperty;var o$1=(e,l)=>t(e,"name",{value:l,configurable:true});var n$2=typeof globalThis<"u"?globalThis:typeof window<"u"?window:typeof global<"u"?global:typeof self<"u"?self:{};function f(e){return e&&e.__esModule&&Object.prototype.hasOwnProperty.call(e,"default")?e.default:e}o$1(f,"getDefaultExportFromCjs");

var Os=Object.defineProperty;var fi=i=>{throw TypeError(i)};var n$1=(i,o)=>Os(i,"name",{value:o,configurable:true});var ci=(i,o,a)=>o.has(i)||fi("Cannot "+a);var O=(i,o,a)=>(ci(i,o,"read from private field"),a?a.call(i):o.get(i)),be=(i,o,a)=>o.has(i)?fi("Cannot add the same private member more than once"):o instanceof WeakSet?o.add(i):o.set(i,a),X=(i,o,a,f)=>(ci(i,o,"write to private field"),o.set(i,a),a);var ve,zt,bt,Cr,ze,It,Ft,mt,ee,yt,He,Ve,gt;function Us(i){if(!/^data:/i.test(i))throw new TypeError('`uri` does not appear to be a Data URI (must begin with "data:")');i=i.replace(/\r?\n/g,"");const o=i.indexOf(",");if(o===-1||o<=4)throw new TypeError("malformed data: URI");const a=i.substring(5,o).split(";");let f="",l=false;const p=a[0]||"text/plain";let h=p;for(let A=1;A<a.length;A++)a[A]==="base64"?l=true:a[A]&&(h+=`;${a[A]}`,a[A].indexOf("charset=")===0&&(f=a[A].substring(8)));!a[0]&&!f.length&&(h+=";charset=US-ASCII",f="US-ASCII");const S=l?"base64":"ascii",v=unescape(i.substring(o+1)),w=Buffer.from(v,S);return w.type=p,w.typeFull=h,w.charset=f,w}n$1(Us,"dataUriToBuffer");var pi={},kt={exports:{}};/**
 * @license
 * web-streams-polyfill v3.3.3
 * Copyright 2024 Mattias Buelens, Diwank Singh Tomer and other contributors.
 * This code is released under the MIT license.
 * SPDX-License-Identifier: MIT
 */var xs=kt.exports,bi;function Ns(){return bi||(bi=1,function(i,o){(function(a,f){f(o);})(xs,function(a){function f(){}n$1(f,"noop");function l(e){return typeof e=="object"&&e!==null||typeof e=="function"}n$1(l,"typeIsObject");const p=f;function h(e,t){try{Object.defineProperty(e,"name",{value:t,configurable:!0});}catch{}}n$1(h,"setFunctionName");const S=Promise,v=Promise.prototype.then,w=Promise.reject.bind(S);function A(e){return new S(e)}n$1(A,"newPromise");function T(e){return A(t=>t(e))}n$1(T,"promiseResolvedWith");function b(e){return w(e)}n$1(b,"promiseRejectedWith");function q(e,t,r){return v.call(e,t,r)}n$1(q,"PerformPromiseThen");function g(e,t,r){q(q(e,t,r),void 0,p);}n$1(g,"uponPromise");function V(e,t){g(e,t);}n$1(V,"uponFulfillment");function I(e,t){g(e,void 0,t);}n$1(I,"uponRejection");function F(e,t,r){return q(e,t,r)}n$1(F,"transformPromiseWith");function Q(e){q(e,void 0,p);}n$1(Q,"setPromiseIsHandledToTrue");let ge=n$1(e=>{if(typeof queueMicrotask=="function")ge=queueMicrotask;else {const t=T(void 0);ge=n$1(r=>q(t,r),"_queueMicrotask");}return ge(e)},"_queueMicrotask");function z(e,t,r){if(typeof e!="function")throw new TypeError("Argument is not a function");return Function.prototype.apply.call(e,t,r)}n$1(z,"reflectCall");function j(e,t,r){try{return T(z(e,t,r))}catch(s){return b(s)}}n$1(j,"promiseCall");const U=16384,bn=class bn{constructor(){this._cursor=0,this._size=0,this._front={_elements:[],_next:void 0},this._back=this._front,this._cursor=0,this._size=0;}get length(){return this._size}push(t){const r=this._back;let s=r;r._elements.length===U-1&&(s={_elements:[],_next:void 0}),r._elements.push(t),s!==r&&(this._back=s,r._next=s),++this._size;}shift(){const t=this._front;let r=t;const s=this._cursor;let u=s+1;const c=t._elements,d=c[s];return u===U&&(r=t._next,u=0),--this._size,this._cursor=u,t!==r&&(this._front=r),c[s]=void 0,d}forEach(t){let r=this._cursor,s=this._front,u=s._elements;for(;(r!==u.length||s._next!==void 0)&&!(r===u.length&&(s=s._next,u=s._elements,r=0,u.length===0));)t(u[r]),++r;}peek(){const t=this._front,r=this._cursor;return t._elements[r]}};n$1(bn,"SimpleQueue");let D=bn;const jt=Symbol("[[AbortSteps]]"),Qn=Symbol("[[ErrorSteps]]"),Ar=Symbol("[[CancelSteps]]"),Br=Symbol("[[PullSteps]]"),kr=Symbol("[[ReleaseSteps]]");function Yn(e,t){e._ownerReadableStream=t,t._reader=e,t._state==="readable"?qr(e):t._state==="closed"?xi(e):Gn(e,t._storedError);}n$1(Yn,"ReadableStreamReaderGenericInitialize");function Wr(e,t){const r=e._ownerReadableStream;return ie(r,t)}n$1(Wr,"ReadableStreamReaderGenericCancel");function _e(e){const t=e._ownerReadableStream;t._state==="readable"?Or(e,new TypeError("Reader was released and can no longer be used to monitor the stream's closedness")):Ni(e,new TypeError("Reader was released and can no longer be used to monitor the stream's closedness")),t._readableStreamController[kr](),t._reader=void 0,e._ownerReadableStream=void 0;}n$1(_e,"ReadableStreamReaderGenericRelease");function Lt(e){return new TypeError("Cannot "+e+" a stream using a released reader")}n$1(Lt,"readerLockException");function qr(e){e._closedPromise=A((t,r)=>{e._closedPromise_resolve=t,e._closedPromise_reject=r;});}n$1(qr,"defaultReaderClosedPromiseInitialize");function Gn(e,t){qr(e),Or(e,t);}n$1(Gn,"defaultReaderClosedPromiseInitializeAsRejected");function xi(e){qr(e),Zn(e);}n$1(xi,"defaultReaderClosedPromiseInitializeAsResolved");function Or(e,t){e._closedPromise_reject!==void 0&&(Q(e._closedPromise),e._closedPromise_reject(t),e._closedPromise_resolve=void 0,e._closedPromise_reject=void 0);}n$1(Or,"defaultReaderClosedPromiseReject");function Ni(e,t){Gn(e,t);}n$1(Ni,"defaultReaderClosedPromiseResetToRejected");function Zn(e){e._closedPromise_resolve!==void 0&&(e._closedPromise_resolve(void 0),e._closedPromise_resolve=void 0,e._closedPromise_reject=void 0);}n$1(Zn,"defaultReaderClosedPromiseResolve");const Kn=Number.isFinite||function(e){return typeof e=="number"&&isFinite(e)},Hi=Math.trunc||function(e){return e<0?Math.ceil(e):Math.floor(e)};function Vi(e){return typeof e=="object"||typeof e=="function"}n$1(Vi,"isDictionary");function ue(e,t){if(e!==void 0&&!Vi(e))throw new TypeError(`${t} is not an object.`)}n$1(ue,"assertDictionary");function Z(e,t){if(typeof e!="function")throw new TypeError(`${t} is not a function.`)}n$1(Z,"assertFunction");function Qi(e){return typeof e=="object"&&e!==null||typeof e=="function"}n$1(Qi,"isObject");function Jn(e,t){if(!Qi(e))throw new TypeError(`${t} is not an object.`)}n$1(Jn,"assertObject");function Se(e,t,r){if(e===void 0)throw new TypeError(`Parameter ${t} is required in '${r}'.`)}n$1(Se,"assertRequiredArgument");function zr(e,t,r){if(e===void 0)throw new TypeError(`${t} is required in '${r}'.`)}n$1(zr,"assertRequiredField");function Ir(e){return Number(e)}n$1(Ir,"convertUnrestrictedDouble");function Xn(e){return e===0?0:e}n$1(Xn,"censorNegativeZero");function Yi(e){return Xn(Hi(e))}n$1(Yi,"integerPart");function Fr(e,t){const s=Number.MAX_SAFE_INTEGER;let u=Number(e);if(u=Xn(u),!Kn(u))throw new TypeError(`${t} is not a finite number`);if(u=Yi(u),u<0||u>s)throw new TypeError(`${t} is outside the accepted range of 0 to ${s}, inclusive`);return !Kn(u)||u===0?0:u}n$1(Fr,"convertUnsignedLongLongWithEnforceRange");function jr(e,t){if(!We(e))throw new TypeError(`${t} is not a ReadableStream.`)}n$1(jr,"assertReadableStream");function Qe(e){return new fe(e)}n$1(Qe,"AcquireReadableStreamDefaultReader");function eo(e,t){e._reader._readRequests.push(t);}n$1(eo,"ReadableStreamAddReadRequest");function Lr(e,t,r){const u=e._reader._readRequests.shift();r?u._closeSteps():u._chunkSteps(t);}n$1(Lr,"ReadableStreamFulfillReadRequest");function $t(e){return e._reader._readRequests.length}n$1($t,"ReadableStreamGetNumReadRequests");function to(e){const t=e._reader;return !(t===void 0||!Ee(t))}n$1(to,"ReadableStreamHasDefaultReader");const mn=class mn{constructor(t){if(Se(t,1,"ReadableStreamDefaultReader"),jr(t,"First parameter"),qe(t))throw new TypeError("This stream has already been locked for exclusive reading by another reader");Yn(this,t),this._readRequests=new D;}get closed(){return Ee(this)?this._closedPromise:b(Dt("closed"))}cancel(t=void 0){return Ee(this)?this._ownerReadableStream===void 0?b(Lt("cancel")):Wr(this,t):b(Dt("cancel"))}read(){if(!Ee(this))return b(Dt("read"));if(this._ownerReadableStream===void 0)return b(Lt("read from"));let t,r;const s=A((c,d)=>{t=c,r=d;});return _t(this,{_chunkSteps:n$1(c=>t({value:c,done:false}),"_chunkSteps"),_closeSteps:n$1(()=>t({value:void 0,done:true}),"_closeSteps"),_errorSteps:n$1(c=>r(c),"_errorSteps")}),s}releaseLock(){if(!Ee(this))throw Dt("releaseLock");this._ownerReadableStream!==void 0&&Gi(this);}};n$1(mn,"ReadableStreamDefaultReader");let fe=mn;Object.defineProperties(fe.prototype,{cancel:{enumerable:true},read:{enumerable:true},releaseLock:{enumerable:true},closed:{enumerable:true}}),h(fe.prototype.cancel,"cancel"),h(fe.prototype.read,"read"),h(fe.prototype.releaseLock,"releaseLock"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(fe.prototype,Symbol.toStringTag,{value:"ReadableStreamDefaultReader",configurable:true});function Ee(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_readRequests")?false:e instanceof fe}n$1(Ee,"IsReadableStreamDefaultReader");function _t(e,t){const r=e._ownerReadableStream;r._disturbed=true,r._state==="closed"?t._closeSteps():r._state==="errored"?t._errorSteps(r._storedError):r._readableStreamController[Br](t);}n$1(_t,"ReadableStreamDefaultReaderRead");function Gi(e){_e(e);const t=new TypeError("Reader was released");ro(e,t);}n$1(Gi,"ReadableStreamDefaultReaderRelease");function ro(e,t){const r=e._readRequests;e._readRequests=new D,r.forEach(s=>{s._errorSteps(t);});}n$1(ro,"ReadableStreamDefaultReaderErrorReadRequests");function Dt(e){return new TypeError(`ReadableStreamDefaultReader.prototype.${e} can only be used on a ReadableStreamDefaultReader`)}n$1(Dt,"defaultReaderBrandCheckException");const Zi=Object.getPrototypeOf(Object.getPrototypeOf(async function*(){}).prototype),yn=class yn{constructor(t,r){this._ongoingPromise=void 0,this._isFinished=false,this._reader=t,this._preventCancel=r;}next(){const t=n$1(()=>this._nextSteps(),"nextSteps");return this._ongoingPromise=this._ongoingPromise?F(this._ongoingPromise,t,t):t(),this._ongoingPromise}return(t){const r=n$1(()=>this._returnSteps(t),"returnSteps");return this._ongoingPromise?F(this._ongoingPromise,r,r):r()}_nextSteps(){if(this._isFinished)return Promise.resolve({value:void 0,done:true});const t=this._reader;let r,s;const u=A((d,m)=>{r=d,s=m;});return _t(t,{_chunkSteps:n$1(d=>{this._ongoingPromise=void 0,ge(()=>r({value:d,done:false}));},"_chunkSteps"),_closeSteps:n$1(()=>{this._ongoingPromise=void 0,this._isFinished=true,_e(t),r({value:void 0,done:true});},"_closeSteps"),_errorSteps:n$1(d=>{this._ongoingPromise=void 0,this._isFinished=true,_e(t),s(d);},"_errorSteps")}),u}_returnSteps(t){if(this._isFinished)return Promise.resolve({value:t,done:true});this._isFinished=true;const r=this._reader;if(!this._preventCancel){const s=Wr(r,t);return _e(r),F(s,()=>({value:t,done:true}))}return _e(r),T({value:t,done:true})}};n$1(yn,"ReadableStreamAsyncIteratorImpl");let Mt=yn;const no={next(){return oo(this)?this._asyncIteratorImpl.next():b(io("next"))},return(e){return oo(this)?this._asyncIteratorImpl.return(e):b(io("return"))}};Object.setPrototypeOf(no,Zi);function Ki(e,t){const r=Qe(e),s=new Mt(r,t),u=Object.create(no);return u._asyncIteratorImpl=s,u}n$1(Ki,"AcquireReadableStreamAsyncIterator");function oo(e){if(!l(e)||!Object.prototype.hasOwnProperty.call(e,"_asyncIteratorImpl"))return  false;try{return e._asyncIteratorImpl instanceof Mt}catch{return  false}}n$1(oo,"IsReadableStreamAsyncIterator");function io(e){return new TypeError(`ReadableStreamAsyncIterator.${e} can only be used on a ReadableSteamAsyncIterator`)}n$1(io,"streamAsyncIteratorBrandCheckException");const ao=Number.isNaN||function(e){return e!==e};var $r,Dr,Mr;function St(e){return e.slice()}n$1(St,"CreateArrayFromList");function so(e,t,r,s,u){new Uint8Array(e).set(new Uint8Array(r,s,u),t);}n$1(so,"CopyDataBlockBytes");let we=n$1(e=>(typeof e.transfer=="function"?we=n$1(t=>t.transfer(),"TransferArrayBuffer"):typeof structuredClone=="function"?we=n$1(t=>structuredClone(t,{transfer:[t]}),"TransferArrayBuffer"):we=n$1(t=>t,"TransferArrayBuffer"),we(e)),"TransferArrayBuffer"),Ae=n$1(e=>(typeof e.detached=="boolean"?Ae=n$1(t=>t.detached,"IsDetachedBuffer"):Ae=n$1(t=>t.byteLength===0,"IsDetachedBuffer"),Ae(e)),"IsDetachedBuffer");function lo(e,t,r){if(e.slice)return e.slice(t,r);const s=r-t,u=new ArrayBuffer(s);return so(u,0,e,t,s),u}n$1(lo,"ArrayBufferSlice");function Ut(e,t){const r=e[t];if(r!=null){if(typeof r!="function")throw new TypeError(`${String(t)} is not a function`);return r}}n$1(Ut,"GetMethod");function Ji(e){const t={[Symbol.iterator]:()=>e.iterator},r=async function*(){return yield*t}(),s=r.next;return {iterator:r,nextMethod:s,done:false}}n$1(Ji,"CreateAsyncFromSyncIterator");const Ur=(Mr=($r=Symbol.asyncIterator)!==null&&$r!==void 0?$r:(Dr=Symbol.for)===null||Dr===void 0?void 0:Dr.call(Symbol,"Symbol.asyncIterator"))!==null&&Mr!==void 0?Mr:"@@asyncIterator";function uo(e,t="sync",r){if(r===void 0)if(t==="async"){if(r=Ut(e,Ur),r===void 0){const c=Ut(e,Symbol.iterator),d=uo(e,"sync",c);return Ji(d)}}else r=Ut(e,Symbol.iterator);if(r===void 0)throw new TypeError("The object is not iterable");const s=z(r,e,[]);if(!l(s))throw new TypeError("The iterator method must return an object");const u=s.next;return {iterator:s,nextMethod:u,done:false}}n$1(uo,"GetIterator");function Xi(e){const t=z(e.nextMethod,e.iterator,[]);if(!l(t))throw new TypeError("The iterator.next() method must return an object");return t}n$1(Xi,"IteratorNext");function ea(e){return !!e.done}n$1(ea,"IteratorComplete");function ta(e){return e.value}n$1(ta,"IteratorValue");function ra(e){return !(typeof e!="number"||ao(e)||e<0)}n$1(ra,"IsNonNegativeNumber");function fo(e){const t=lo(e.buffer,e.byteOffset,e.byteOffset+e.byteLength);return new Uint8Array(t)}n$1(fo,"CloneAsUint8Array");function xr(e){const t=e._queue.shift();return e._queueTotalSize-=t.size,e._queueTotalSize<0&&(e._queueTotalSize=0),t.value}n$1(xr,"DequeueValue");function Nr(e,t,r){if(!ra(r)||r===1/0)throw new RangeError("Size must be a finite, non-NaN, non-negative number.");e._queue.push({value:t,size:r}),e._queueTotalSize+=r;}n$1(Nr,"EnqueueValueWithSize");function na(e){return e._queue.peek().value}n$1(na,"PeekQueueValue");function Be(e){e._queue=new D,e._queueTotalSize=0;}n$1(Be,"ResetQueue");function co(e){return e===DataView}n$1(co,"isDataViewConstructor");function oa(e){return co(e.constructor)}n$1(oa,"isDataView");function ia(e){return co(e)?1:e.BYTES_PER_ELEMENT}n$1(ia,"arrayBufferViewElementSize");const gn=class gn{constructor(){throw new TypeError("Illegal constructor")}get view(){if(!Hr(this))throw Zr("view");return this._view}respond(t){if(!Hr(this))throw Zr("respond");if(Se(t,1,"respond"),t=Fr(t,"First parameter"),this._associatedReadableByteStreamController===void 0)throw new TypeError("This BYOB request has been invalidated");if(Ae(this._view.buffer))throw new TypeError("The BYOB request's buffer has been detached and so cannot be used as a response");Vt(this._associatedReadableByteStreamController,t);}respondWithNewView(t){if(!Hr(this))throw Zr("respondWithNewView");if(Se(t,1,"respondWithNewView"),!ArrayBuffer.isView(t))throw new TypeError("You can only respond with array buffer views");if(this._associatedReadableByteStreamController===void 0)throw new TypeError("This BYOB request has been invalidated");if(Ae(t.buffer))throw new TypeError("The given view's buffer has been detached and so cannot be used as a response");Qt(this._associatedReadableByteStreamController,t);}};n$1(gn,"ReadableStreamBYOBRequest");let Re=gn;Object.defineProperties(Re.prototype,{respond:{enumerable:true},respondWithNewView:{enumerable:true},view:{enumerable:true}}),h(Re.prototype.respond,"respond"),h(Re.prototype.respondWithNewView,"respondWithNewView"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(Re.prototype,Symbol.toStringTag,{value:"ReadableStreamBYOBRequest",configurable:true});const _n=class _n{constructor(){throw new TypeError("Illegal constructor")}get byobRequest(){if(!Ie(this))throw Rt("byobRequest");return Gr(this)}get desiredSize(){if(!Ie(this))throw Rt("desiredSize");return Ro(this)}close(){if(!Ie(this))throw Rt("close");if(this._closeRequested)throw new TypeError("The stream has already been closed; do not close it again!");const t=this._controlledReadableByteStream._state;if(t!=="readable")throw new TypeError(`The stream (in ${t} state) is not in the readable state and cannot be closed`);wt(this);}enqueue(t){if(!Ie(this))throw Rt("enqueue");if(Se(t,1,"enqueue"),!ArrayBuffer.isView(t))throw new TypeError("chunk must be an array buffer view");if(t.byteLength===0)throw new TypeError("chunk must have non-zero byteLength");if(t.buffer.byteLength===0)throw new TypeError("chunk's buffer must have non-zero byteLength");if(this._closeRequested)throw new TypeError("stream is closed or draining");const r=this._controlledReadableByteStream._state;if(r!=="readable")throw new TypeError(`The stream (in ${r} state) is not in the readable state and cannot be enqueued to`);Ht(this,t);}error(t=void 0){if(!Ie(this))throw Rt("error");K(this,t);}[Ar](t){ho(this),Be(this);const r=this._cancelAlgorithm(t);return Nt(this),r}[Br](t){const r=this._controlledReadableByteStream;if(this._queueTotalSize>0){wo(this,t);return}const s=this._autoAllocateChunkSize;if(s!==void 0){let u;try{u=new ArrayBuffer(s);}catch(d){t._errorSteps(d);return}const c={buffer:u,bufferByteLength:s,byteOffset:0,byteLength:s,bytesFilled:0,minimumFill:1,elementSize:1,viewConstructor:Uint8Array,readerType:"default"};this._pendingPullIntos.push(c);}eo(r,t),Fe(this);}[kr](){if(this._pendingPullIntos.length>0){const t=this._pendingPullIntos.peek();t.readerType="none",this._pendingPullIntos=new D,this._pendingPullIntos.push(t);}}};n$1(_n,"ReadableByteStreamController");let te=_n;Object.defineProperties(te.prototype,{close:{enumerable:true},enqueue:{enumerable:true},error:{enumerable:true},byobRequest:{enumerable:true},desiredSize:{enumerable:true}}),h(te.prototype.close,"close"),h(te.prototype.enqueue,"enqueue"),h(te.prototype.error,"error"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(te.prototype,Symbol.toStringTag,{value:"ReadableByteStreamController",configurable:true});function Ie(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_controlledReadableByteStream")?false:e instanceof te}n$1(Ie,"IsReadableByteStreamController");function Hr(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_associatedReadableByteStreamController")?false:e instanceof Re}n$1(Hr,"IsReadableStreamBYOBRequest");function Fe(e){if(!fa(e))return;if(e._pulling){e._pullAgain=true;return}e._pulling=true;const r=e._pullAlgorithm();g(r,()=>(e._pulling=false,e._pullAgain&&(e._pullAgain=false,Fe(e)),null),s=>(K(e,s),null));}n$1(Fe,"ReadableByteStreamControllerCallPullIfNeeded");function ho(e){Qr(e),e._pendingPullIntos=new D;}n$1(ho,"ReadableByteStreamControllerClearPendingPullIntos");function Vr(e,t){let r=false;e._state==="closed"&&(r=true);const s=po(t);t.readerType==="default"?Lr(e,s,r):ma(e,s,r);}n$1(Vr,"ReadableByteStreamControllerCommitPullIntoDescriptor");function po(e){const t=e.bytesFilled,r=e.elementSize;return new e.viewConstructor(e.buffer,e.byteOffset,t/r)}n$1(po,"ReadableByteStreamControllerConvertPullIntoDescriptor");function xt(e,t,r,s){e._queue.push({buffer:t,byteOffset:r,byteLength:s}),e._queueTotalSize+=s;}n$1(xt,"ReadableByteStreamControllerEnqueueChunkToQueue");function bo(e,t,r,s){let u;try{u=lo(t,r,r+s);}catch(c){throw K(e,c),c}xt(e,u,0,s);}n$1(bo,"ReadableByteStreamControllerEnqueueClonedChunkToQueue");function mo(e,t){t.bytesFilled>0&&bo(e,t.buffer,t.byteOffset,t.bytesFilled),Ye(e);}n$1(mo,"ReadableByteStreamControllerEnqueueDetachedPullIntoToQueue");function yo(e,t){const r=Math.min(e._queueTotalSize,t.byteLength-t.bytesFilled),s=t.bytesFilled+r;let u=r,c=false;const d=s%t.elementSize,m=s-d;m>=t.minimumFill&&(u=m-t.bytesFilled,c=true);const R=e._queue;for(;u>0;){const y=R.peek(),C=Math.min(u,y.byteLength),P=t.byteOffset+t.bytesFilled;so(t.buffer,P,y.buffer,y.byteOffset,C),y.byteLength===C?R.shift():(y.byteOffset+=C,y.byteLength-=C),e._queueTotalSize-=C,go(e,C,t),u-=C;}return c}n$1(yo,"ReadableByteStreamControllerFillPullIntoDescriptorFromQueue");function go(e,t,r){r.bytesFilled+=t;}n$1(go,"ReadableByteStreamControllerFillHeadPullIntoDescriptor");function _o(e){e._queueTotalSize===0&&e._closeRequested?(Nt(e),At(e._controlledReadableByteStream)):Fe(e);}n$1(_o,"ReadableByteStreamControllerHandleQueueDrain");function Qr(e){e._byobRequest!==null&&(e._byobRequest._associatedReadableByteStreamController=void 0,e._byobRequest._view=null,e._byobRequest=null);}n$1(Qr,"ReadableByteStreamControllerInvalidateBYOBRequest");function Yr(e){for(;e._pendingPullIntos.length>0;){if(e._queueTotalSize===0)return;const t=e._pendingPullIntos.peek();yo(e,t)&&(Ye(e),Vr(e._controlledReadableByteStream,t));}}n$1(Yr,"ReadableByteStreamControllerProcessPullIntoDescriptorsUsingQueue");function aa(e){const t=e._controlledReadableByteStream._reader;for(;t._readRequests.length>0;){if(e._queueTotalSize===0)return;const r=t._readRequests.shift();wo(e,r);}}n$1(aa,"ReadableByteStreamControllerProcessReadRequestsUsingQueue");function sa(e,t,r,s){const u=e._controlledReadableByteStream,c=t.constructor,d=ia(c),{byteOffset:m,byteLength:R}=t,y=r*d;let C;try{C=we(t.buffer);}catch(B){s._errorSteps(B);return}const P={buffer:C,bufferByteLength:C.byteLength,byteOffset:m,byteLength:R,bytesFilled:0,minimumFill:y,elementSize:d,viewConstructor:c,readerType:"byob"};if(e._pendingPullIntos.length>0){e._pendingPullIntos.push(P),Po(u,s);return}if(u._state==="closed"){const B=new c(P.buffer,P.byteOffset,0);s._closeSteps(B);return}if(e._queueTotalSize>0){if(yo(e,P)){const B=po(P);_o(e),s._chunkSteps(B);return}if(e._closeRequested){const B=new TypeError("Insufficient bytes to fill elements in the given buffer");K(e,B),s._errorSteps(B);return}}e._pendingPullIntos.push(P),Po(u,s),Fe(e);}n$1(sa,"ReadableByteStreamControllerPullInto");function la(e,t){t.readerType==="none"&&Ye(e);const r=e._controlledReadableByteStream;if(Kr(r))for(;vo(r)>0;){const s=Ye(e);Vr(r,s);}}n$1(la,"ReadableByteStreamControllerRespondInClosedState");function ua(e,t,r){if(go(e,t,r),r.readerType==="none"){mo(e,r),Yr(e);return}if(r.bytesFilled<r.minimumFill)return;Ye(e);const s=r.bytesFilled%r.elementSize;if(s>0){const u=r.byteOffset+r.bytesFilled;bo(e,r.buffer,u-s,s);}r.bytesFilled-=s,Vr(e._controlledReadableByteStream,r),Yr(e);}n$1(ua,"ReadableByteStreamControllerRespondInReadableState");function So(e,t){const r=e._pendingPullIntos.peek();Qr(e),e._controlledReadableByteStream._state==="closed"?la(e,r):ua(e,t,r),Fe(e);}n$1(So,"ReadableByteStreamControllerRespondInternal");function Ye(e){return e._pendingPullIntos.shift()}n$1(Ye,"ReadableByteStreamControllerShiftPendingPullInto");function fa(e){const t=e._controlledReadableByteStream;return t._state!=="readable"||e._closeRequested||!e._started?false:!!(to(t)&&$t(t)>0||Kr(t)&&vo(t)>0||Ro(e)>0)}n$1(fa,"ReadableByteStreamControllerShouldCallPull");function Nt(e){e._pullAlgorithm=void 0,e._cancelAlgorithm=void 0;}n$1(Nt,"ReadableByteStreamControllerClearAlgorithms");function wt(e){const t=e._controlledReadableByteStream;if(!(e._closeRequested||t._state!=="readable")){if(e._queueTotalSize>0){e._closeRequested=true;return}if(e._pendingPullIntos.length>0){const r=e._pendingPullIntos.peek();if(r.bytesFilled%r.elementSize!==0){const s=new TypeError("Insufficient bytes to fill elements in the given buffer");throw K(e,s),s}}Nt(e),At(t);}}n$1(wt,"ReadableByteStreamControllerClose");function Ht(e,t){const r=e._controlledReadableByteStream;if(e._closeRequested||r._state!=="readable")return;const{buffer:s,byteOffset:u,byteLength:c}=t;if(Ae(s))throw new TypeError("chunk's buffer is detached and so cannot be enqueued");const d=we(s);if(e._pendingPullIntos.length>0){const m=e._pendingPullIntos.peek();if(Ae(m.buffer))throw new TypeError("The BYOB request's buffer has been detached and so cannot be filled with an enqueued chunk");Qr(e),m.buffer=we(m.buffer),m.readerType==="none"&&mo(e,m);}if(to(r))if(aa(e),$t(r)===0)xt(e,d,u,c);else {e._pendingPullIntos.length>0&&Ye(e);const m=new Uint8Array(d,u,c);Lr(r,m,false);}else Kr(r)?(xt(e,d,u,c),Yr(e)):xt(e,d,u,c);Fe(e);}n$1(Ht,"ReadableByteStreamControllerEnqueue");function K(e,t){const r=e._controlledReadableByteStream;r._state==="readable"&&(ho(e),Be(e),Nt(e),Zo(r,t));}n$1(K,"ReadableByteStreamControllerError");function wo(e,t){const r=e._queue.shift();e._queueTotalSize-=r.byteLength,_o(e);const s=new Uint8Array(r.buffer,r.byteOffset,r.byteLength);t._chunkSteps(s);}n$1(wo,"ReadableByteStreamControllerFillReadRequestFromQueue");function Gr(e){if(e._byobRequest===null&&e._pendingPullIntos.length>0){const t=e._pendingPullIntos.peek(),r=new Uint8Array(t.buffer,t.byteOffset+t.bytesFilled,t.byteLength-t.bytesFilled),s=Object.create(Re.prototype);da(s,e,r),e._byobRequest=s;}return e._byobRequest}n$1(Gr,"ReadableByteStreamControllerGetBYOBRequest");function Ro(e){const t=e._controlledReadableByteStream._state;return t==="errored"?null:t==="closed"?0:e._strategyHWM-e._queueTotalSize}n$1(Ro,"ReadableByteStreamControllerGetDesiredSize");function Vt(e,t){const r=e._pendingPullIntos.peek();if(e._controlledReadableByteStream._state==="closed"){if(t!==0)throw new TypeError("bytesWritten must be 0 when calling respond() on a closed stream")}else {if(t===0)throw new TypeError("bytesWritten must be greater than 0 when calling respond() on a readable stream");if(r.bytesFilled+t>r.byteLength)throw new RangeError("bytesWritten out of range")}r.buffer=we(r.buffer),So(e,t);}n$1(Vt,"ReadableByteStreamControllerRespond");function Qt(e,t){const r=e._pendingPullIntos.peek();if(e._controlledReadableByteStream._state==="closed"){if(t.byteLength!==0)throw new TypeError("The view's length must be 0 when calling respondWithNewView() on a closed stream")}else if(t.byteLength===0)throw new TypeError("The view's length must be greater than 0 when calling respondWithNewView() on a readable stream");if(r.byteOffset+r.bytesFilled!==t.byteOffset)throw new RangeError("The region specified by view does not match byobRequest");if(r.bufferByteLength!==t.buffer.byteLength)throw new RangeError("The buffer of view has different capacity than byobRequest");if(r.bytesFilled+t.byteLength>r.byteLength)throw new RangeError("The region specified by view is larger than byobRequest");const u=t.byteLength;r.buffer=we(t.buffer),So(e,u);}n$1(Qt,"ReadableByteStreamControllerRespondWithNewView");function To(e,t,r,s,u,c,d){t._controlledReadableByteStream=e,t._pullAgain=false,t._pulling=false,t._byobRequest=null,t._queue=t._queueTotalSize=void 0,Be(t),t._closeRequested=false,t._started=false,t._strategyHWM=c,t._pullAlgorithm=s,t._cancelAlgorithm=u,t._autoAllocateChunkSize=d,t._pendingPullIntos=new D,e._readableStreamController=t;const m=r();g(T(m),()=>(t._started=true,Fe(t),null),R=>(K(t,R),null));}n$1(To,"SetUpReadableByteStreamController");function ca(e,t,r){const s=Object.create(te.prototype);let u,c,d;t.start!==void 0?u=n$1(()=>t.start(s),"startAlgorithm"):u=n$1(()=>{},"startAlgorithm"),t.pull!==void 0?c=n$1(()=>t.pull(s),"pullAlgorithm"):c=n$1(()=>T(void 0),"pullAlgorithm"),t.cancel!==void 0?d=n$1(R=>t.cancel(R),"cancelAlgorithm"):d=n$1(()=>T(void 0),"cancelAlgorithm");const m=t.autoAllocateChunkSize;if(m===0)throw new TypeError("autoAllocateChunkSize must be greater than 0");To(e,s,u,c,d,r,m);}n$1(ca,"SetUpReadableByteStreamControllerFromUnderlyingSource");function da(e,t,r){e._associatedReadableByteStreamController=t,e._view=r;}n$1(da,"SetUpReadableStreamBYOBRequest");function Zr(e){return new TypeError(`ReadableStreamBYOBRequest.prototype.${e} can only be used on a ReadableStreamBYOBRequest`)}n$1(Zr,"byobRequestBrandCheckException");function Rt(e){return new TypeError(`ReadableByteStreamController.prototype.${e} can only be used on a ReadableByteStreamController`)}n$1(Rt,"byteStreamControllerBrandCheckException");function ha(e,t){ue(e,t);const r=e?.mode;return {mode:r===void 0?void 0:pa(r,`${t} has member 'mode' that`)}}n$1(ha,"convertReaderOptions");function pa(e,t){if(e=`${e}`,e!=="byob")throw new TypeError(`${t} '${e}' is not a valid enumeration value for ReadableStreamReaderMode`);return e}n$1(pa,"convertReadableStreamReaderMode");function ba(e,t){var r;ue(e,t);const s=(r=e?.min)!==null&&r!==void 0?r:1;return {min:Fr(s,`${t} has member 'min' that`)}}n$1(ba,"convertByobReadOptions");function Co(e){return new ce(e)}n$1(Co,"AcquireReadableStreamBYOBReader");function Po(e,t){e._reader._readIntoRequests.push(t);}n$1(Po,"ReadableStreamAddReadIntoRequest");function ma(e,t,r){const u=e._reader._readIntoRequests.shift();r?u._closeSteps(t):u._chunkSteps(t);}n$1(ma,"ReadableStreamFulfillReadIntoRequest");function vo(e){return e._reader._readIntoRequests.length}n$1(vo,"ReadableStreamGetNumReadIntoRequests");function Kr(e){const t=e._reader;return !(t===void 0||!je(t))}n$1(Kr,"ReadableStreamHasBYOBReader");const Sn=class Sn{constructor(t){if(Se(t,1,"ReadableStreamBYOBReader"),jr(t,"First parameter"),qe(t))throw new TypeError("This stream has already been locked for exclusive reading by another reader");if(!Ie(t._readableStreamController))throw new TypeError("Cannot construct a ReadableStreamBYOBReader for a stream not constructed with a byte source");Yn(this,t),this._readIntoRequests=new D;}get closed(){return je(this)?this._closedPromise:b(Yt("closed"))}cancel(t=void 0){return je(this)?this._ownerReadableStream===void 0?b(Lt("cancel")):Wr(this,t):b(Yt("cancel"))}read(t,r={}){if(!je(this))return b(Yt("read"));if(!ArrayBuffer.isView(t))return b(new TypeError("view must be an array buffer view"));if(t.byteLength===0)return b(new TypeError("view must have non-zero byteLength"));if(t.buffer.byteLength===0)return b(new TypeError("view's buffer must have non-zero byteLength"));if(Ae(t.buffer))return b(new TypeError("view's buffer has been detached"));let s;try{s=ba(r,"options");}catch(y){return b(y)}const u=s.min;if(u===0)return b(new TypeError("options.min must be greater than 0"));if(oa(t)){if(u>t.byteLength)return b(new RangeError("options.min must be less than or equal to view's byteLength"))}else if(u>t.length)return b(new RangeError("options.min must be less than or equal to view's length"));if(this._ownerReadableStream===void 0)return b(Lt("read from"));let c,d;const m=A((y,C)=>{c=y,d=C;});return Eo(this,t,u,{_chunkSteps:n$1(y=>c({value:y,done:false}),"_chunkSteps"),_closeSteps:n$1(y=>c({value:y,done:true}),"_closeSteps"),_errorSteps:n$1(y=>d(y),"_errorSteps")}),m}releaseLock(){if(!je(this))throw Yt("releaseLock");this._ownerReadableStream!==void 0&&ya(this);}};n$1(Sn,"ReadableStreamBYOBReader");let ce=Sn;Object.defineProperties(ce.prototype,{cancel:{enumerable:true},read:{enumerable:true},releaseLock:{enumerable:true},closed:{enumerable:true}}),h(ce.prototype.cancel,"cancel"),h(ce.prototype.read,"read"),h(ce.prototype.releaseLock,"releaseLock"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(ce.prototype,Symbol.toStringTag,{value:"ReadableStreamBYOBReader",configurable:true});function je(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_readIntoRequests")?false:e instanceof ce}n$1(je,"IsReadableStreamBYOBReader");function Eo(e,t,r,s){const u=e._ownerReadableStream;u._disturbed=true,u._state==="errored"?s._errorSteps(u._storedError):sa(u._readableStreamController,t,r,s);}n$1(Eo,"ReadableStreamBYOBReaderRead");function ya(e){_e(e);const t=new TypeError("Reader was released");Ao(e,t);}n$1(ya,"ReadableStreamBYOBReaderRelease");function Ao(e,t){const r=e._readIntoRequests;e._readIntoRequests=new D,r.forEach(s=>{s._errorSteps(t);});}n$1(Ao,"ReadableStreamBYOBReaderErrorReadIntoRequests");function Yt(e){return new TypeError(`ReadableStreamBYOBReader.prototype.${e} can only be used on a ReadableStreamBYOBReader`)}n$1(Yt,"byobReaderBrandCheckException");function Tt(e,t){const{highWaterMark:r}=e;if(r===void 0)return t;if(ao(r)||r<0)throw new RangeError("Invalid highWaterMark");return r}n$1(Tt,"ExtractHighWaterMark");function Gt(e){const{size:t}=e;return t||(()=>1)}n$1(Gt,"ExtractSizeAlgorithm");function Zt(e,t){ue(e,t);const r=e?.highWaterMark,s=e?.size;return {highWaterMark:r===void 0?void 0:Ir(r),size:s===void 0?void 0:ga(s,`${t} has member 'size' that`)}}n$1(Zt,"convertQueuingStrategy");function ga(e,t){return Z(e,t),r=>Ir(e(r))}n$1(ga,"convertQueuingStrategySize");function _a(e,t){ue(e,t);const r=e?.abort,s=e?.close,u=e?.start,c=e?.type,d=e?.write;return {abort:r===void 0?void 0:Sa(r,e,`${t} has member 'abort' that`),close:s===void 0?void 0:wa(s,e,`${t} has member 'close' that`),start:u===void 0?void 0:Ra(u,e,`${t} has member 'start' that`),write:d===void 0?void 0:Ta(d,e,`${t} has member 'write' that`),type:c}}n$1(_a,"convertUnderlyingSink");function Sa(e,t,r){return Z(e,r),s=>j(e,t,[s])}n$1(Sa,"convertUnderlyingSinkAbortCallback");function wa(e,t,r){return Z(e,r),()=>j(e,t,[])}n$1(wa,"convertUnderlyingSinkCloseCallback");function Ra(e,t,r){return Z(e,r),s=>z(e,t,[s])}n$1(Ra,"convertUnderlyingSinkStartCallback");function Ta(e,t,r){return Z(e,r),(s,u)=>j(e,t,[s,u])}n$1(Ta,"convertUnderlyingSinkWriteCallback");function Bo(e,t){if(!Ge(e))throw new TypeError(`${t} is not a WritableStream.`)}n$1(Bo,"assertWritableStream");function Ca(e){if(typeof e!="object"||e===null)return  false;try{return typeof e.aborted=="boolean"}catch{return  false}}n$1(Ca,"isAbortSignal");const Pa=typeof AbortController=="function";function va(){if(Pa)return new AbortController}n$1(va,"createAbortController");const wn=class wn{constructor(t={},r={}){t===void 0?t=null:Jn(t,"First parameter");const s=Zt(r,"Second parameter"),u=_a(t,"First parameter");if(Wo(this),u.type!==void 0)throw new RangeError("Invalid type is specified");const d=Gt(s),m=Tt(s,1);Da(this,u,m,d);}get locked(){if(!Ge(this))throw tr("locked");return Ze(this)}abort(t=void 0){return Ge(this)?Ze(this)?b(new TypeError("Cannot abort a stream that already has a writer")):Kt(this,t):b(tr("abort"))}close(){return Ge(this)?Ze(this)?b(new TypeError("Cannot close a stream that already has a writer")):he(this)?b(new TypeError("Cannot close an already-closing stream")):qo(this):b(tr("close"))}getWriter(){if(!Ge(this))throw tr("getWriter");return ko(this)}};n$1(wn,"WritableStream");let de=wn;Object.defineProperties(de.prototype,{abort:{enumerable:true},close:{enumerable:true},getWriter:{enumerable:true},locked:{enumerable:true}}),h(de.prototype.abort,"abort"),h(de.prototype.close,"close"),h(de.prototype.getWriter,"getWriter"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(de.prototype,Symbol.toStringTag,{value:"WritableStream",configurable:true});function ko(e){return new re(e)}n$1(ko,"AcquireWritableStreamDefaultWriter");function Ea(e,t,r,s,u=1,c=()=>1){const d=Object.create(de.prototype);Wo(d);const m=Object.create(ke.prototype);return Lo(d,m,e,t,r,s,u,c),d}n$1(Ea,"CreateWritableStream");function Wo(e){e._state="writable",e._storedError=void 0,e._writer=void 0,e._writableStreamController=void 0,e._writeRequests=new D,e._inFlightWriteRequest=void 0,e._closeRequest=void 0,e._inFlightCloseRequest=void 0,e._pendingAbortRequest=void 0,e._backpressure=false;}n$1(Wo,"InitializeWritableStream");function Ge(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_writableStreamController")?false:e instanceof de}n$1(Ge,"IsWritableStream");function Ze(e){return e._writer!==void 0}n$1(Ze,"IsWritableStreamLocked");function Kt(e,t){var r;if(e._state==="closed"||e._state==="errored")return T(void 0);e._writableStreamController._abortReason=t,(r=e._writableStreamController._abortController)===null||r===void 0||r.abort(t);const s=e._state;if(s==="closed"||s==="errored")return T(void 0);if(e._pendingAbortRequest!==void 0)return e._pendingAbortRequest._promise;let u=false;s==="erroring"&&(u=true,t=void 0);const c=A((d,m)=>{e._pendingAbortRequest={_promise:void 0,_resolve:d,_reject:m,_reason:t,_wasAlreadyErroring:u};});return e._pendingAbortRequest._promise=c,u||Xr(e,t),c}n$1(Kt,"WritableStreamAbort");function qo(e){const t=e._state;if(t==="closed"||t==="errored")return b(new TypeError(`The stream (in ${t} state) is not in the writable state and cannot be closed`));const r=A((u,c)=>{const d={_resolve:u,_reject:c};e._closeRequest=d;}),s=e._writer;return s!==void 0&&e._backpressure&&t==="writable"&&ln(s),Ma(e._writableStreamController),r}n$1(qo,"WritableStreamClose");function Aa(e){return A((r,s)=>{const u={_resolve:r,_reject:s};e._writeRequests.push(u);})}n$1(Aa,"WritableStreamAddWriteRequest");function Jr(e,t){if(e._state==="writable"){Xr(e,t);return}en(e);}n$1(Jr,"WritableStreamDealWithRejection");function Xr(e,t){const r=e._writableStreamController;e._state="erroring",e._storedError=t;const s=e._writer;s!==void 0&&zo(s,t),!Oa(e)&&r._started&&en(e);}n$1(Xr,"WritableStreamStartErroring");function en(e){e._state="errored",e._writableStreamController[Qn]();const t=e._storedError;if(e._writeRequests.forEach(u=>{u._reject(t);}),e._writeRequests=new D,e._pendingAbortRequest===void 0){Jt(e);return}const r=e._pendingAbortRequest;if(e._pendingAbortRequest=void 0,r._wasAlreadyErroring){r._reject(t),Jt(e);return}const s=e._writableStreamController[jt](r._reason);g(s,()=>(r._resolve(),Jt(e),null),u=>(r._reject(u),Jt(e),null));}n$1(en,"WritableStreamFinishErroring");function Ba(e){e._inFlightWriteRequest._resolve(void 0),e._inFlightWriteRequest=void 0;}n$1(Ba,"WritableStreamFinishInFlightWrite");function ka(e,t){e._inFlightWriteRequest._reject(t),e._inFlightWriteRequest=void 0,Jr(e,t);}n$1(ka,"WritableStreamFinishInFlightWriteWithError");function Wa(e){e._inFlightCloseRequest._resolve(void 0),e._inFlightCloseRequest=void 0,e._state==="erroring"&&(e._storedError=void 0,e._pendingAbortRequest!==void 0&&(e._pendingAbortRequest._resolve(),e._pendingAbortRequest=void 0)),e._state="closed";const r=e._writer;r!==void 0&&Uo(r);}n$1(Wa,"WritableStreamFinishInFlightClose");function qa(e,t){e._inFlightCloseRequest._reject(t),e._inFlightCloseRequest=void 0,e._pendingAbortRequest!==void 0&&(e._pendingAbortRequest._reject(t),e._pendingAbortRequest=void 0),Jr(e,t);}n$1(qa,"WritableStreamFinishInFlightCloseWithError");function he(e){return !(e._closeRequest===void 0&&e._inFlightCloseRequest===void 0)}n$1(he,"WritableStreamCloseQueuedOrInFlight");function Oa(e){return !(e._inFlightWriteRequest===void 0&&e._inFlightCloseRequest===void 0)}n$1(Oa,"WritableStreamHasOperationMarkedInFlight");function za(e){e._inFlightCloseRequest=e._closeRequest,e._closeRequest=void 0;}n$1(za,"WritableStreamMarkCloseRequestInFlight");function Ia(e){e._inFlightWriteRequest=e._writeRequests.shift();}n$1(Ia,"WritableStreamMarkFirstWriteRequestInFlight");function Jt(e){e._closeRequest!==void 0&&(e._closeRequest._reject(e._storedError),e._closeRequest=void 0);const t=e._writer;t!==void 0&&an(t,e._storedError);}n$1(Jt,"WritableStreamRejectCloseAndClosedPromiseIfNeeded");function tn(e,t){const r=e._writer;r!==void 0&&t!==e._backpressure&&(t?Ya(r):ln(r)),e._backpressure=t;}n$1(tn,"WritableStreamUpdateBackpressure");const Rn=class Rn{constructor(t){if(Se(t,1,"WritableStreamDefaultWriter"),Bo(t,"First parameter"),Ze(t))throw new TypeError("This stream has already been locked for exclusive writing by another writer");this._ownerWritableStream=t,t._writer=this;const r=t._state;if(r==="writable")!he(t)&&t._backpressure?nr(this):xo(this),rr(this);else if(r==="erroring")sn(this,t._storedError),rr(this);else if(r==="closed")xo(this),Va(this);else {const s=t._storedError;sn(this,s),Mo(this,s);}}get closed(){return Le(this)?this._closedPromise:b($e("closed"))}get desiredSize(){if(!Le(this))throw $e("desiredSize");if(this._ownerWritableStream===void 0)throw Pt("desiredSize");return $a(this)}get ready(){return Le(this)?this._readyPromise:b($e("ready"))}abort(t=void 0){return Le(this)?this._ownerWritableStream===void 0?b(Pt("abort")):Fa(this,t):b($e("abort"))}close(){if(!Le(this))return b($e("close"));const t=this._ownerWritableStream;return t===void 0?b(Pt("close")):he(t)?b(new TypeError("Cannot close an already-closing stream")):Oo(this)}releaseLock(){if(!Le(this))throw $e("releaseLock");this._ownerWritableStream!==void 0&&Io(this);}write(t=void 0){return Le(this)?this._ownerWritableStream===void 0?b(Pt("write to")):Fo(this,t):b($e("write"))}};n$1(Rn,"WritableStreamDefaultWriter");let re=Rn;Object.defineProperties(re.prototype,{abort:{enumerable:true},close:{enumerable:true},releaseLock:{enumerable:true},write:{enumerable:true},closed:{enumerable:true},desiredSize:{enumerable:true},ready:{enumerable:true}}),h(re.prototype.abort,"abort"),h(re.prototype.close,"close"),h(re.prototype.releaseLock,"releaseLock"),h(re.prototype.write,"write"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(re.prototype,Symbol.toStringTag,{value:"WritableStreamDefaultWriter",configurable:true});function Le(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_ownerWritableStream")?false:e instanceof re}n$1(Le,"IsWritableStreamDefaultWriter");function Fa(e,t){const r=e._ownerWritableStream;return Kt(r,t)}n$1(Fa,"WritableStreamDefaultWriterAbort");function Oo(e){const t=e._ownerWritableStream;return qo(t)}n$1(Oo,"WritableStreamDefaultWriterClose");function ja(e){const t=e._ownerWritableStream,r=t._state;return he(t)||r==="closed"?T(void 0):r==="errored"?b(t._storedError):Oo(e)}n$1(ja,"WritableStreamDefaultWriterCloseWithErrorPropagation");function La(e,t){e._closedPromiseState==="pending"?an(e,t):Qa(e,t);}n$1(La,"WritableStreamDefaultWriterEnsureClosedPromiseRejected");function zo(e,t){e._readyPromiseState==="pending"?No(e,t):Ga(e,t);}n$1(zo,"WritableStreamDefaultWriterEnsureReadyPromiseRejected");function $a(e){const t=e._ownerWritableStream,r=t._state;return r==="errored"||r==="erroring"?null:r==="closed"?0:$o(t._writableStreamController)}n$1($a,"WritableStreamDefaultWriterGetDesiredSize");function Io(e){const t=e._ownerWritableStream,r=new TypeError("Writer was released and can no longer be used to monitor the stream's closedness");zo(e,r),La(e,r),t._writer=void 0,e._ownerWritableStream=void 0;}n$1(Io,"WritableStreamDefaultWriterRelease");function Fo(e,t){const r=e._ownerWritableStream,s=r._writableStreamController,u=Ua(s,t);if(r!==e._ownerWritableStream)return b(Pt("write to"));const c=r._state;if(c==="errored")return b(r._storedError);if(he(r)||c==="closed")return b(new TypeError("The stream is closing or closed and cannot be written to"));if(c==="erroring")return b(r._storedError);const d=Aa(r);return xa(s,t,u),d}n$1(Fo,"WritableStreamDefaultWriterWrite");const jo={},Tn=class Tn{constructor(){throw new TypeError("Illegal constructor")}get abortReason(){if(!rn(this))throw on("abortReason");return this._abortReason}get signal(){if(!rn(this))throw on("signal");if(this._abortController===void 0)throw new TypeError("WritableStreamDefaultController.prototype.signal is not supported");return this._abortController.signal}error(t=void 0){if(!rn(this))throw on("error");this._controlledWritableStream._state==="writable"&&Do(this,t);}[jt](t){const r=this._abortAlgorithm(t);return Xt(this),r}[Qn](){Be(this);}};n$1(Tn,"WritableStreamDefaultController");let ke=Tn;Object.defineProperties(ke.prototype,{abortReason:{enumerable:true},signal:{enumerable:true},error:{enumerable:true}}),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(ke.prototype,Symbol.toStringTag,{value:"WritableStreamDefaultController",configurable:true});function rn(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_controlledWritableStream")?false:e instanceof ke}n$1(rn,"IsWritableStreamDefaultController");function Lo(e,t,r,s,u,c,d,m){t._controlledWritableStream=e,e._writableStreamController=t,t._queue=void 0,t._queueTotalSize=void 0,Be(t),t._abortReason=void 0,t._abortController=va(),t._started=false,t._strategySizeAlgorithm=m,t._strategyHWM=d,t._writeAlgorithm=s,t._closeAlgorithm=u,t._abortAlgorithm=c;const R=nn(t);tn(e,R);const y=r(),C=T(y);g(C,()=>(t._started=true,er(t),null),P=>(t._started=true,Jr(e,P),null));}n$1(Lo,"SetUpWritableStreamDefaultController");function Da(e,t,r,s){const u=Object.create(ke.prototype);let c,d,m,R;t.start!==void 0?c=n$1(()=>t.start(u),"startAlgorithm"):c=n$1(()=>{},"startAlgorithm"),t.write!==void 0?d=n$1(y=>t.write(y,u),"writeAlgorithm"):d=n$1(()=>T(void 0),"writeAlgorithm"),t.close!==void 0?m=n$1(()=>t.close(),"closeAlgorithm"):m=n$1(()=>T(void 0),"closeAlgorithm"),t.abort!==void 0?R=n$1(y=>t.abort(y),"abortAlgorithm"):R=n$1(()=>T(void 0),"abortAlgorithm"),Lo(e,u,c,d,m,R,r,s);}n$1(Da,"SetUpWritableStreamDefaultControllerFromUnderlyingSink");function Xt(e){e._writeAlgorithm=void 0,e._closeAlgorithm=void 0,e._abortAlgorithm=void 0,e._strategySizeAlgorithm=void 0;}n$1(Xt,"WritableStreamDefaultControllerClearAlgorithms");function Ma(e){Nr(e,jo,0),er(e);}n$1(Ma,"WritableStreamDefaultControllerClose");function Ua(e,t){try{return e._strategySizeAlgorithm(t)}catch(r){return Ct(e,r),1}}n$1(Ua,"WritableStreamDefaultControllerGetChunkSize");function $o(e){return e._strategyHWM-e._queueTotalSize}n$1($o,"WritableStreamDefaultControllerGetDesiredSize");function xa(e,t,r){try{Nr(e,t,r);}catch(u){Ct(e,u);return}const s=e._controlledWritableStream;if(!he(s)&&s._state==="writable"){const u=nn(e);tn(s,u);}er(e);}n$1(xa,"WritableStreamDefaultControllerWrite");function er(e){const t=e._controlledWritableStream;if(!e._started||t._inFlightWriteRequest!==void 0)return;if(t._state==="erroring"){en(t);return}if(e._queue.length===0)return;const s=na(e);s===jo?Na(e):Ha(e,s);}n$1(er,"WritableStreamDefaultControllerAdvanceQueueIfNeeded");function Ct(e,t){e._controlledWritableStream._state==="writable"&&Do(e,t);}n$1(Ct,"WritableStreamDefaultControllerErrorIfNeeded");function Na(e){const t=e._controlledWritableStream;za(t),xr(e);const r=e._closeAlgorithm();Xt(e),g(r,()=>(Wa(t),null),s=>(qa(t,s),null));}n$1(Na,"WritableStreamDefaultControllerProcessClose");function Ha(e,t){const r=e._controlledWritableStream;Ia(r);const s=e._writeAlgorithm(t);g(s,()=>{Ba(r);const u=r._state;if(xr(e),!he(r)&&u==="writable"){const c=nn(e);tn(r,c);}return er(e),null},u=>(r._state==="writable"&&Xt(e),ka(r,u),null));}n$1(Ha,"WritableStreamDefaultControllerProcessWrite");function nn(e){return $o(e)<=0}n$1(nn,"WritableStreamDefaultControllerGetBackpressure");function Do(e,t){const r=e._controlledWritableStream;Xt(e),Xr(r,t);}n$1(Do,"WritableStreamDefaultControllerError");function tr(e){return new TypeError(`WritableStream.prototype.${e} can only be used on a WritableStream`)}n$1(tr,"streamBrandCheckException$2");function on(e){return new TypeError(`WritableStreamDefaultController.prototype.${e} can only be used on a WritableStreamDefaultController`)}n$1(on,"defaultControllerBrandCheckException$2");function $e(e){return new TypeError(`WritableStreamDefaultWriter.prototype.${e} can only be used on a WritableStreamDefaultWriter`)}n$1($e,"defaultWriterBrandCheckException");function Pt(e){return new TypeError("Cannot "+e+" a stream using a released writer")}n$1(Pt,"defaultWriterLockException");function rr(e){e._closedPromise=A((t,r)=>{e._closedPromise_resolve=t,e._closedPromise_reject=r,e._closedPromiseState="pending";});}n$1(rr,"defaultWriterClosedPromiseInitialize");function Mo(e,t){rr(e),an(e,t);}n$1(Mo,"defaultWriterClosedPromiseInitializeAsRejected");function Va(e){rr(e),Uo(e);}n$1(Va,"defaultWriterClosedPromiseInitializeAsResolved");function an(e,t){e._closedPromise_reject!==void 0&&(Q(e._closedPromise),e._closedPromise_reject(t),e._closedPromise_resolve=void 0,e._closedPromise_reject=void 0,e._closedPromiseState="rejected");}n$1(an,"defaultWriterClosedPromiseReject");function Qa(e,t){Mo(e,t);}n$1(Qa,"defaultWriterClosedPromiseResetToRejected");function Uo(e){e._closedPromise_resolve!==void 0&&(e._closedPromise_resolve(void 0),e._closedPromise_resolve=void 0,e._closedPromise_reject=void 0,e._closedPromiseState="resolved");}n$1(Uo,"defaultWriterClosedPromiseResolve");function nr(e){e._readyPromise=A((t,r)=>{e._readyPromise_resolve=t,e._readyPromise_reject=r;}),e._readyPromiseState="pending";}n$1(nr,"defaultWriterReadyPromiseInitialize");function sn(e,t){nr(e),No(e,t);}n$1(sn,"defaultWriterReadyPromiseInitializeAsRejected");function xo(e){nr(e),ln(e);}n$1(xo,"defaultWriterReadyPromiseInitializeAsResolved");function No(e,t){e._readyPromise_reject!==void 0&&(Q(e._readyPromise),e._readyPromise_reject(t),e._readyPromise_resolve=void 0,e._readyPromise_reject=void 0,e._readyPromiseState="rejected");}n$1(No,"defaultWriterReadyPromiseReject");function Ya(e){nr(e);}n$1(Ya,"defaultWriterReadyPromiseReset");function Ga(e,t){sn(e,t);}n$1(Ga,"defaultWriterReadyPromiseResetToRejected");function ln(e){e._readyPromise_resolve!==void 0&&(e._readyPromise_resolve(void 0),e._readyPromise_resolve=void 0,e._readyPromise_reject=void 0,e._readyPromiseState="fulfilled");}n$1(ln,"defaultWriterReadyPromiseResolve");function Za(){if(typeof globalThis<"u")return globalThis;if(typeof self<"u")return self;if(typeof n$2<"u")return n$2}n$1(Za,"getGlobals");const un=Za();function Ka(e){if(!(typeof e=="function"||typeof e=="object")||e.name!=="DOMException")return  false;try{return new e,!0}catch{return  false}}n$1(Ka,"isDOMExceptionConstructor");function Ja(){const e=un?.DOMException;return Ka(e)?e:void 0}n$1(Ja,"getFromGlobal");function Xa(){const e=n$1(function(r,s){this.message=r||"",this.name=s||"Error",Error.captureStackTrace&&Error.captureStackTrace(this,this.constructor);},"DOMException");return h(e,"DOMException"),e.prototype=Object.create(Error.prototype),Object.defineProperty(e.prototype,"constructor",{value:e,writable:true,configurable:true}),e}n$1(Xa,"createPolyfill");const es=Ja()||Xa();function Ho(e,t,r,s,u,c){const d=Qe(e),m=ko(t);e._disturbed=true;let R=false,y=T(void 0);return A((C,P)=>{let B;if(c!==void 0){if(B=n$1(()=>{const _=c.reason!==void 0?c.reason:new es("Aborted","AbortError"),E=[];s||E.push(()=>t._state==="writable"?Kt(t,_):T(void 0)),u||E.push(()=>e._state==="readable"?ie(e,_):T(void 0)),N(()=>Promise.all(E.map(k=>k())),true,_);},"abortAlgorithm"),c.aborted){B();return}c.addEventListener("abort",B);}function ae(){return A((_,E)=>{function k(Y){Y?_():q(nt(),k,E);}n$1(k,"next"),k(false);})}n$1(ae,"pipeLoop");function nt(){return R?T(true):q(m._readyPromise,()=>A((_,E)=>{_t(d,{_chunkSteps:n$1(k=>{y=q(Fo(m,k),void 0,f),_(false);},"_chunkSteps"),_closeSteps:n$1(()=>_(true),"_closeSteps"),_errorSteps:E});}))}if(n$1(nt,"pipeStep"),Te(e,d._closedPromise,_=>(s?J(true,_):N(()=>Kt(t,_),true,_),null)),Te(t,m._closedPromise,_=>(u?J(true,_):N(()=>ie(e,_),true,_),null)),x(e,d._closedPromise,()=>(r?J():N(()=>ja(m)),null)),he(t)||t._state==="closed"){const _=new TypeError("the destination writable stream closed before all data could be piped to it");u?J(true,_):N(()=>ie(e,_),true,_);}Q(ae());function Oe(){const _=y;return q(y,()=>_!==y?Oe():void 0)}n$1(Oe,"waitForWritesToFinish");function Te(_,E,k){_._state==="errored"?k(_._storedError):I(E,k);}n$1(Te,"isOrBecomesErrored");function x(_,E,k){_._state==="closed"?k():V(E,k);}n$1(x,"isOrBecomesClosed");function N(_,E,k){if(R)return;R=true,t._state==="writable"&&!he(t)?V(Oe(),Y):Y();function Y(){return g(_(),()=>Ce(E,k),ot=>Ce(true,ot)),null}n$1(Y,"doTheRest");}n$1(N,"shutdownWithAction");function J(_,E){R||(R=true,t._state==="writable"&&!he(t)?V(Oe(),()=>Ce(_,E)):Ce(_,E));}n$1(J,"shutdown");function Ce(_,E){return Io(m),_e(d),c!==void 0&&c.removeEventListener("abort",B),_?P(E):C(void 0),null}n$1(Ce,"finalize");})}n$1(Ho,"ReadableStreamPipeTo");const Cn=class Cn{constructor(){throw new TypeError("Illegal constructor")}get desiredSize(){if(!or(this))throw ar("desiredSize");return fn(this)}close(){if(!or(this))throw ar("close");if(!Je(this))throw new TypeError("The stream is not in a state that permits close");De(this);}enqueue(t=void 0){if(!or(this))throw ar("enqueue");if(!Je(this))throw new TypeError("The stream is not in a state that permits enqueue");return Ke(this,t)}error(t=void 0){if(!or(this))throw ar("error");oe(this,t);}[Ar](t){Be(this);const r=this._cancelAlgorithm(t);return ir(this),r}[Br](t){const r=this._controlledReadableStream;if(this._queue.length>0){const s=xr(this);this._closeRequested&&this._queue.length===0?(ir(this),At(r)):vt(this),t._chunkSteps(s);}else eo(r,t),vt(this);}[kr](){}};n$1(Cn,"ReadableStreamDefaultController");let ne=Cn;Object.defineProperties(ne.prototype,{close:{enumerable:true},enqueue:{enumerable:true},error:{enumerable:true},desiredSize:{enumerable:true}}),h(ne.prototype.close,"close"),h(ne.prototype.enqueue,"enqueue"),h(ne.prototype.error,"error"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(ne.prototype,Symbol.toStringTag,{value:"ReadableStreamDefaultController",configurable:true});function or(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_controlledReadableStream")?false:e instanceof ne}n$1(or,"IsReadableStreamDefaultController");function vt(e){if(!Vo(e))return;if(e._pulling){e._pullAgain=true;return}e._pulling=true;const r=e._pullAlgorithm();g(r,()=>(e._pulling=false,e._pullAgain&&(e._pullAgain=false,vt(e)),null),s=>(oe(e,s),null));}n$1(vt,"ReadableStreamDefaultControllerCallPullIfNeeded");function Vo(e){const t=e._controlledReadableStream;return !Je(e)||!e._started?false:!!(qe(t)&&$t(t)>0||fn(e)>0)}n$1(Vo,"ReadableStreamDefaultControllerShouldCallPull");function ir(e){e._pullAlgorithm=void 0,e._cancelAlgorithm=void 0,e._strategySizeAlgorithm=void 0;}n$1(ir,"ReadableStreamDefaultControllerClearAlgorithms");function De(e){if(!Je(e))return;const t=e._controlledReadableStream;e._closeRequested=true,e._queue.length===0&&(ir(e),At(t));}n$1(De,"ReadableStreamDefaultControllerClose");function Ke(e,t){if(!Je(e))return;const r=e._controlledReadableStream;if(qe(r)&&$t(r)>0)Lr(r,t,false);else {let s;try{s=e._strategySizeAlgorithm(t);}catch(u){throw oe(e,u),u}try{Nr(e,t,s);}catch(u){throw oe(e,u),u}}vt(e);}n$1(Ke,"ReadableStreamDefaultControllerEnqueue");function oe(e,t){const r=e._controlledReadableStream;r._state==="readable"&&(Be(e),ir(e),Zo(r,t));}n$1(oe,"ReadableStreamDefaultControllerError");function fn(e){const t=e._controlledReadableStream._state;return t==="errored"?null:t==="closed"?0:e._strategyHWM-e._queueTotalSize}n$1(fn,"ReadableStreamDefaultControllerGetDesiredSize");function ts(e){return !Vo(e)}n$1(ts,"ReadableStreamDefaultControllerHasBackpressure");function Je(e){const t=e._controlledReadableStream._state;return !e._closeRequested&&t==="readable"}n$1(Je,"ReadableStreamDefaultControllerCanCloseOrEnqueue");function Qo(e,t,r,s,u,c,d){t._controlledReadableStream=e,t._queue=void 0,t._queueTotalSize=void 0,Be(t),t._started=false,t._closeRequested=false,t._pullAgain=false,t._pulling=false,t._strategySizeAlgorithm=d,t._strategyHWM=c,t._pullAlgorithm=s,t._cancelAlgorithm=u,e._readableStreamController=t;const m=r();g(T(m),()=>(t._started=true,vt(t),null),R=>(oe(t,R),null));}n$1(Qo,"SetUpReadableStreamDefaultController");function rs(e,t,r,s){const u=Object.create(ne.prototype);let c,d,m;t.start!==void 0?c=n$1(()=>t.start(u),"startAlgorithm"):c=n$1(()=>{},"startAlgorithm"),t.pull!==void 0?d=n$1(()=>t.pull(u),"pullAlgorithm"):d=n$1(()=>T(void 0),"pullAlgorithm"),t.cancel!==void 0?m=n$1(R=>t.cancel(R),"cancelAlgorithm"):m=n$1(()=>T(void 0),"cancelAlgorithm"),Qo(e,u,c,d,m,r,s);}n$1(rs,"SetUpReadableStreamDefaultControllerFromUnderlyingSource");function ar(e){return new TypeError(`ReadableStreamDefaultController.prototype.${e} can only be used on a ReadableStreamDefaultController`)}n$1(ar,"defaultControllerBrandCheckException$1");function ns(e,t){return Ie(e._readableStreamController)?is(e):os(e)}n$1(ns,"ReadableStreamTee");function os(e,t){const r=Qe(e);let s=false,u=false,c=false,d=false,m,R,y,C,P;const B=A(x=>{P=x;});function ae(){return s?(u=true,T(void 0)):(s=true,_t(r,{_chunkSteps:n$1(N=>{ge(()=>{u=false;const J=N,Ce=N;c||Ke(y._readableStreamController,J),d||Ke(C._readableStreamController,Ce),s=false,u&&ae();});},"_chunkSteps"),_closeSteps:n$1(()=>{s=false,c||De(y._readableStreamController),d||De(C._readableStreamController),(!c||!d)&&P(void 0);},"_closeSteps"),_errorSteps:n$1(()=>{s=false;},"_errorSteps")}),T(void 0))}n$1(ae,"pullAlgorithm");function nt(x){if(c=true,m=x,d){const N=St([m,R]),J=ie(e,N);P(J);}return B}n$1(nt,"cancel1Algorithm");function Oe(x){if(d=true,R=x,c){const N=St([m,R]),J=ie(e,N);P(J);}return B}n$1(Oe,"cancel2Algorithm");function Te(){}return n$1(Te,"startAlgorithm"),y=Et(Te,ae,nt),C=Et(Te,ae,Oe),I(r._closedPromise,x=>(oe(y._readableStreamController,x),oe(C._readableStreamController,x),(!c||!d)&&P(void 0),null)),[y,C]}n$1(os,"ReadableStreamDefaultTee");function is(e){let t=Qe(e),r=false,s=false,u=false,c=false,d=false,m,R,y,C,P;const B=A(_=>{P=_;});function ae(_){I(_._closedPromise,E=>(_!==t||(K(y._readableStreamController,E),K(C._readableStreamController,E),(!c||!d)&&P(void 0)),null));}n$1(ae,"forwardReaderError");function nt(){je(t)&&(_e(t),t=Qe(e),ae(t)),_t(t,{_chunkSteps:n$1(E=>{ge(()=>{s=false,u=false;const k=E;let Y=E;if(!c&&!d)try{Y=fo(E);}catch(ot){K(y._readableStreamController,ot),K(C._readableStreamController,ot),P(ie(e,ot));return}c||Ht(y._readableStreamController,k),d||Ht(C._readableStreamController,Y),r=false,s?Te():u&&x();});},"_chunkSteps"),_closeSteps:n$1(()=>{r=false,c||wt(y._readableStreamController),d||wt(C._readableStreamController),y._readableStreamController._pendingPullIntos.length>0&&Vt(y._readableStreamController,0),C._readableStreamController._pendingPullIntos.length>0&&Vt(C._readableStreamController,0),(!c||!d)&&P(void 0);},"_closeSteps"),_errorSteps:n$1(()=>{r=false;},"_errorSteps")});}n$1(nt,"pullWithDefaultReader");function Oe(_,E){Ee(t)&&(_e(t),t=Co(e),ae(t));const k=E?C:y,Y=E?y:C;Eo(t,_,1,{_chunkSteps:n$1(it=>{ge(()=>{s=false,u=false;const at=E?d:c;if(E?c:d)at||Qt(k._readableStreamController,it);else {let ui;try{ui=fo(it);}catch(kn){K(k._readableStreamController,kn),K(Y._readableStreamController,kn),P(ie(e,kn));return}at||Qt(k._readableStreamController,it),Ht(Y._readableStreamController,ui);}r=false,s?Te():u&&x();});},"_chunkSteps"),_closeSteps:n$1(it=>{r=false;const at=E?d:c,cr=E?c:d;at||wt(k._readableStreamController),cr||wt(Y._readableStreamController),it!==void 0&&(at||Qt(k._readableStreamController,it),!cr&&Y._readableStreamController._pendingPullIntos.length>0&&Vt(Y._readableStreamController,0)),(!at||!cr)&&P(void 0);},"_closeSteps"),_errorSteps:n$1(()=>{r=false;},"_errorSteps")});}n$1(Oe,"pullWithBYOBReader");function Te(){if(r)return s=true,T(void 0);r=true;const _=Gr(y._readableStreamController);return _===null?nt():Oe(_._view,false),T(void 0)}n$1(Te,"pull1Algorithm");function x(){if(r)return u=true,T(void 0);r=true;const _=Gr(C._readableStreamController);return _===null?nt():Oe(_._view,true),T(void 0)}n$1(x,"pull2Algorithm");function N(_){if(c=true,m=_,d){const E=St([m,R]),k=ie(e,E);P(k);}return B}n$1(N,"cancel1Algorithm");function J(_){if(d=true,R=_,c){const E=St([m,R]),k=ie(e,E);P(k);}return B}n$1(J,"cancel2Algorithm");function Ce(){}return n$1(Ce,"startAlgorithm"),y=Go(Ce,Te,N),C=Go(Ce,x,J),ae(t),[y,C]}n$1(is,"ReadableByteStreamTee");function as(e){return l(e)&&typeof e.getReader<"u"}n$1(as,"isReadableStreamLike");function ss(e){return as(e)?us(e.getReader()):ls(e)}n$1(ss,"ReadableStreamFrom");function ls(e){let t;const r=uo(e,"async"),s=f;function u(){let d;try{d=Xi(r);}catch(R){return b(R)}const m=T(d);return F(m,R=>{if(!l(R))throw new TypeError("The promise returned by the iterator.next() method must fulfill with an object");if(ea(R))De(t._readableStreamController);else {const C=ta(R);Ke(t._readableStreamController,C);}})}n$1(u,"pullAlgorithm");function c(d){const m=r.iterator;let R;try{R=Ut(m,"return");}catch(P){return b(P)}if(R===void 0)return T(void 0);let y;try{y=z(R,m,[d]);}catch(P){return b(P)}const C=T(y);return F(C,P=>{if(!l(P))throw new TypeError("The promise returned by the iterator.return() method must fulfill with an object")})}return n$1(c,"cancelAlgorithm"),t=Et(s,u,c,0),t}n$1(ls,"ReadableStreamFromIterable");function us(e){let t;const r=f;function s(){let c;try{c=e.read();}catch(d){return b(d)}return F(c,d=>{if(!l(d))throw new TypeError("The promise returned by the reader.read() method must fulfill with an object");if(d.done)De(t._readableStreamController);else {const m=d.value;Ke(t._readableStreamController,m);}})}n$1(s,"pullAlgorithm");function u(c){try{return T(e.cancel(c))}catch(d){return b(d)}}return n$1(u,"cancelAlgorithm"),t=Et(r,s,u,0),t}n$1(us,"ReadableStreamFromDefaultReader");function fs(e,t){ue(e,t);const r=e,s=r?.autoAllocateChunkSize,u=r?.cancel,c=r?.pull,d=r?.start,m=r?.type;return {autoAllocateChunkSize:s===void 0?void 0:Fr(s,`${t} has member 'autoAllocateChunkSize' that`),cancel:u===void 0?void 0:cs(u,r,`${t} has member 'cancel' that`),pull:c===void 0?void 0:ds(c,r,`${t} has member 'pull' that`),start:d===void 0?void 0:hs(d,r,`${t} has member 'start' that`),type:m===void 0?void 0:ps(m,`${t} has member 'type' that`)}}n$1(fs,"convertUnderlyingDefaultOrByteSource");function cs(e,t,r){return Z(e,r),s=>j(e,t,[s])}n$1(cs,"convertUnderlyingSourceCancelCallback");function ds(e,t,r){return Z(e,r),s=>j(e,t,[s])}n$1(ds,"convertUnderlyingSourcePullCallback");function hs(e,t,r){return Z(e,r),s=>z(e,t,[s])}n$1(hs,"convertUnderlyingSourceStartCallback");function ps(e,t){if(e=`${e}`,e!=="bytes")throw new TypeError(`${t} '${e}' is not a valid enumeration value for ReadableStreamType`);return e}n$1(ps,"convertReadableStreamType");function bs(e,t){return ue(e,t),{preventCancel:!!e?.preventCancel}}n$1(bs,"convertIteratorOptions");function Yo(e,t){ue(e,t);const r=e?.preventAbort,s=e?.preventCancel,u=e?.preventClose,c=e?.signal;return c!==void 0&&ms(c,`${t} has member 'signal' that`),{preventAbort:!!r,preventCancel:!!s,preventClose:!!u,signal:c}}n$1(Yo,"convertPipeOptions");function ms(e,t){if(!Ca(e))throw new TypeError(`${t} is not an AbortSignal.`)}n$1(ms,"assertAbortSignal");function ys(e,t){ue(e,t);const r=e?.readable;zr(r,"readable","ReadableWritablePair"),jr(r,`${t} has member 'readable' that`);const s=e?.writable;return zr(s,"writable","ReadableWritablePair"),Bo(s,`${t} has member 'writable' that`),{readable:r,writable:s}}n$1(ys,"convertReadableWritablePair");const Pn=class Pn{constructor(t={},r={}){t===void 0?t=null:Jn(t,"First parameter");const s=Zt(r,"Second parameter"),u=fs(t,"First parameter");if(cn(this),u.type==="bytes"){if(s.size!==void 0)throw new RangeError("The strategy for a byte stream cannot have a size function");const c=Tt(s,0);ca(this,u,c);}else {const c=Gt(s),d=Tt(s,1);rs(this,u,d,c);}}get locked(){if(!We(this))throw Me("locked");return qe(this)}cancel(t=void 0){return We(this)?qe(this)?b(new TypeError("Cannot cancel a stream that already has a reader")):ie(this,t):b(Me("cancel"))}getReader(t=void 0){if(!We(this))throw Me("getReader");return ha(t,"First parameter").mode===void 0?Qe(this):Co(this)}pipeThrough(t,r={}){if(!We(this))throw Me("pipeThrough");Se(t,1,"pipeThrough");const s=ys(t,"First parameter"),u=Yo(r,"Second parameter");if(qe(this))throw new TypeError("ReadableStream.prototype.pipeThrough cannot be used on a locked ReadableStream");if(Ze(s.writable))throw new TypeError("ReadableStream.prototype.pipeThrough cannot be used on a locked WritableStream");const c=Ho(this,s.writable,u.preventClose,u.preventAbort,u.preventCancel,u.signal);return Q(c),s.readable}pipeTo(t,r={}){if(!We(this))return b(Me("pipeTo"));if(t===void 0)return b("Parameter 1 is required in 'pipeTo'.");if(!Ge(t))return b(new TypeError("ReadableStream.prototype.pipeTo's first argument must be a WritableStream"));let s;try{s=Yo(r,"Second parameter");}catch(u){return b(u)}return qe(this)?b(new TypeError("ReadableStream.prototype.pipeTo cannot be used on a locked ReadableStream")):Ze(t)?b(new TypeError("ReadableStream.prototype.pipeTo cannot be used on a locked WritableStream")):Ho(this,t,s.preventClose,s.preventAbort,s.preventCancel,s.signal)}tee(){if(!We(this))throw Me("tee");const t=ns(this);return St(t)}values(t=void 0){if(!We(this))throw Me("values");const r=bs(t,"First parameter");return Ki(this,r.preventCancel)}[Ur](t){return this.values(t)}static from(t){return ss(t)}};n$1(Pn,"ReadableStream");let L=Pn;Object.defineProperties(L,{from:{enumerable:true}}),Object.defineProperties(L.prototype,{cancel:{enumerable:true},getReader:{enumerable:true},pipeThrough:{enumerable:true},pipeTo:{enumerable:true},tee:{enumerable:true},values:{enumerable:true},locked:{enumerable:true}}),h(L.from,"from"),h(L.prototype.cancel,"cancel"),h(L.prototype.getReader,"getReader"),h(L.prototype.pipeThrough,"pipeThrough"),h(L.prototype.pipeTo,"pipeTo"),h(L.prototype.tee,"tee"),h(L.prototype.values,"values"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(L.prototype,Symbol.toStringTag,{value:"ReadableStream",configurable:true}),Object.defineProperty(L.prototype,Ur,{value:L.prototype.values,writable:true,configurable:true});function Et(e,t,r,s=1,u=()=>1){const c=Object.create(L.prototype);cn(c);const d=Object.create(ne.prototype);return Qo(c,d,e,t,r,s,u),c}n$1(Et,"CreateReadableStream");function Go(e,t,r){const s=Object.create(L.prototype);cn(s);const u=Object.create(te.prototype);return To(s,u,e,t,r,0,void 0),s}n$1(Go,"CreateReadableByteStream");function cn(e){e._state="readable",e._reader=void 0,e._storedError=void 0,e._disturbed=false;}n$1(cn,"InitializeReadableStream");function We(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_readableStreamController")?false:e instanceof L}n$1(We,"IsReadableStream");function qe(e){return e._reader!==void 0}n$1(qe,"IsReadableStreamLocked");function ie(e,t){if(e._disturbed=true,e._state==="closed")return T(void 0);if(e._state==="errored")return b(e._storedError);At(e);const r=e._reader;if(r!==void 0&&je(r)){const u=r._readIntoRequests;r._readIntoRequests=new D,u.forEach(c=>{c._closeSteps(void 0);});}const s=e._readableStreamController[Ar](t);return F(s,f)}n$1(ie,"ReadableStreamCancel");function At(e){e._state="closed";const t=e._reader;if(t!==void 0&&(Zn(t),Ee(t))){const r=t._readRequests;t._readRequests=new D,r.forEach(s=>{s._closeSteps();});}}n$1(At,"ReadableStreamClose");function Zo(e,t){e._state="errored",e._storedError=t;const r=e._reader;r!==void 0&&(Or(r,t),Ee(r)?ro(r,t):Ao(r,t));}n$1(Zo,"ReadableStreamError");function Me(e){return new TypeError(`ReadableStream.prototype.${e} can only be used on a ReadableStream`)}n$1(Me,"streamBrandCheckException$1");function Ko(e,t){ue(e,t);const r=e?.highWaterMark;return zr(r,"highWaterMark","QueuingStrategyInit"),{highWaterMark:Ir(r)}}n$1(Ko,"convertQueuingStrategyInit");const Jo=n$1(e=>e.byteLength,"byteLengthSizeFunction");h(Jo,"size");const vn=class vn{constructor(t){Se(t,1,"ByteLengthQueuingStrategy"),t=Ko(t,"First parameter"),this._byteLengthQueuingStrategyHighWaterMark=t.highWaterMark;}get highWaterMark(){if(!ei(this))throw Xo("highWaterMark");return this._byteLengthQueuingStrategyHighWaterMark}get size(){if(!ei(this))throw Xo("size");return Jo}};n$1(vn,"ByteLengthQueuingStrategy");let Xe=vn;Object.defineProperties(Xe.prototype,{highWaterMark:{enumerable:true},size:{enumerable:true}}),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(Xe.prototype,Symbol.toStringTag,{value:"ByteLengthQueuingStrategy",configurable:true});function Xo(e){return new TypeError(`ByteLengthQueuingStrategy.prototype.${e} can only be used on a ByteLengthQueuingStrategy`)}n$1(Xo,"byteLengthBrandCheckException");function ei(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_byteLengthQueuingStrategyHighWaterMark")?false:e instanceof Xe}n$1(ei,"IsByteLengthQueuingStrategy");const ti=n$1(()=>1,"countSizeFunction");h(ti,"size");const En=class En{constructor(t){Se(t,1,"CountQueuingStrategy"),t=Ko(t,"First parameter"),this._countQueuingStrategyHighWaterMark=t.highWaterMark;}get highWaterMark(){if(!ni(this))throw ri("highWaterMark");return this._countQueuingStrategyHighWaterMark}get size(){if(!ni(this))throw ri("size");return ti}};n$1(En,"CountQueuingStrategy");let et=En;Object.defineProperties(et.prototype,{highWaterMark:{enumerable:true},size:{enumerable:true}}),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(et.prototype,Symbol.toStringTag,{value:"CountQueuingStrategy",configurable:true});function ri(e){return new TypeError(`CountQueuingStrategy.prototype.${e} can only be used on a CountQueuingStrategy`)}n$1(ri,"countBrandCheckException");function ni(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_countQueuingStrategyHighWaterMark")?false:e instanceof et}n$1(ni,"IsCountQueuingStrategy");function gs(e,t){ue(e,t);const r=e?.cancel,s=e?.flush,u=e?.readableType,c=e?.start,d=e?.transform,m=e?.writableType;return {cancel:r===void 0?void 0:Rs(r,e,`${t} has member 'cancel' that`),flush:s===void 0?void 0:_s(s,e,`${t} has member 'flush' that`),readableType:u,start:c===void 0?void 0:Ss(c,e,`${t} has member 'start' that`),transform:d===void 0?void 0:ws(d,e,`${t} has member 'transform' that`),writableType:m}}n$1(gs,"convertTransformer");function _s(e,t,r){return Z(e,r),s=>j(e,t,[s])}n$1(_s,"convertTransformerFlushCallback");function Ss(e,t,r){return Z(e,r),s=>z(e,t,[s])}n$1(Ss,"convertTransformerStartCallback");function ws(e,t,r){return Z(e,r),(s,u)=>j(e,t,[s,u])}n$1(ws,"convertTransformerTransformCallback");function Rs(e,t,r){return Z(e,r),s=>j(e,t,[s])}n$1(Rs,"convertTransformerCancelCallback");const An=class An{constructor(t={},r={},s={}){t===void 0&&(t=null);const u=Zt(r,"Second parameter"),c=Zt(s,"Third parameter"),d=gs(t,"First parameter");if(d.readableType!==void 0)throw new RangeError("Invalid readableType specified");if(d.writableType!==void 0)throw new RangeError("Invalid writableType specified");const m=Tt(c,0),R=Gt(c),y=Tt(u,1),C=Gt(u);let P;const B=A(ae=>{P=ae;});Ts(this,B,y,C,m,R),Ps(this,d),d.start!==void 0?P(d.start(this._transformStreamController)):P(void 0);}get readable(){if(!oi(this))throw li("readable");return this._readable}get writable(){if(!oi(this))throw li("writable");return this._writable}};n$1(An,"TransformStream");let tt=An;Object.defineProperties(tt.prototype,{readable:{enumerable:true},writable:{enumerable:true}}),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(tt.prototype,Symbol.toStringTag,{value:"TransformStream",configurable:true});function Ts(e,t,r,s,u,c){function d(){return t}n$1(d,"startAlgorithm");function m(B){return As(e,B)}n$1(m,"writeAlgorithm");function R(B){return Bs(e,B)}n$1(R,"abortAlgorithm");function y(){return ks(e)}n$1(y,"closeAlgorithm"),e._writable=Ea(d,m,y,R,r,s);function C(){return Ws(e)}n$1(C,"pullAlgorithm");function P(B){return qs(e,B)}n$1(P,"cancelAlgorithm"),e._readable=Et(d,C,P,u,c),e._backpressure=void 0,e._backpressureChangePromise=void 0,e._backpressureChangePromise_resolve=void 0,sr(e,true),e._transformStreamController=void 0;}n$1(Ts,"InitializeTransformStream");function oi(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_transformStreamController")?false:e instanceof tt}n$1(oi,"IsTransformStream");function ii(e,t){oe(e._readable._readableStreamController,t),dn(e,t);}n$1(ii,"TransformStreamError");function dn(e,t){ur(e._transformStreamController),Ct(e._writable._writableStreamController,t),hn(e);}n$1(dn,"TransformStreamErrorWritableAndUnblockWrite");function hn(e){e._backpressure&&sr(e,false);}n$1(hn,"TransformStreamUnblockWrite");function sr(e,t){e._backpressureChangePromise!==void 0&&e._backpressureChangePromise_resolve(),e._backpressureChangePromise=A(r=>{e._backpressureChangePromise_resolve=r;}),e._backpressure=t;}n$1(sr,"TransformStreamSetBackpressure");const Bn=class Bn{constructor(){throw new TypeError("Illegal constructor")}get desiredSize(){if(!lr(this))throw fr("desiredSize");const t=this._controlledTransformStream._readable._readableStreamController;return fn(t)}enqueue(t=void 0){if(!lr(this))throw fr("enqueue");ai(this,t);}error(t=void 0){if(!lr(this))throw fr("error");vs(this,t);}terminate(){if(!lr(this))throw fr("terminate");Es(this);}};n$1(Bn,"TransformStreamDefaultController");let pe=Bn;Object.defineProperties(pe.prototype,{enqueue:{enumerable:true},error:{enumerable:true},terminate:{enumerable:true},desiredSize:{enumerable:true}}),h(pe.prototype.enqueue,"enqueue"),h(pe.prototype.error,"error"),h(pe.prototype.terminate,"terminate"),typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(pe.prototype,Symbol.toStringTag,{value:"TransformStreamDefaultController",configurable:true});function lr(e){return !l(e)||!Object.prototype.hasOwnProperty.call(e,"_controlledTransformStream")?false:e instanceof pe}n$1(lr,"IsTransformStreamDefaultController");function Cs(e,t,r,s,u){t._controlledTransformStream=e,e._transformStreamController=t,t._transformAlgorithm=r,t._flushAlgorithm=s,t._cancelAlgorithm=u,t._finishPromise=void 0,t._finishPromise_resolve=void 0,t._finishPromise_reject=void 0;}n$1(Cs,"SetUpTransformStreamDefaultController");function Ps(e,t){const r=Object.create(pe.prototype);let s,u,c;t.transform!==void 0?s=n$1(d=>t.transform(d,r),"transformAlgorithm"):s=n$1(d=>{try{return ai(r,d),T(void 0)}catch(m){return b(m)}},"transformAlgorithm"),t.flush!==void 0?u=n$1(()=>t.flush(r),"flushAlgorithm"):u=n$1(()=>T(void 0),"flushAlgorithm"),t.cancel!==void 0?c=n$1(d=>t.cancel(d),"cancelAlgorithm"):c=n$1(()=>T(void 0),"cancelAlgorithm"),Cs(e,r,s,u,c);}n$1(Ps,"SetUpTransformStreamDefaultControllerFromTransformer");function ur(e){e._transformAlgorithm=void 0,e._flushAlgorithm=void 0,e._cancelAlgorithm=void 0;}n$1(ur,"TransformStreamDefaultControllerClearAlgorithms");function ai(e,t){const r=e._controlledTransformStream,s=r._readable._readableStreamController;if(!Je(s))throw new TypeError("Readable side is not in a state that permits enqueue");try{Ke(s,t);}catch(c){throw dn(r,c),r._readable._storedError}ts(s)!==r._backpressure&&sr(r,true);}n$1(ai,"TransformStreamDefaultControllerEnqueue");function vs(e,t){ii(e._controlledTransformStream,t);}n$1(vs,"TransformStreamDefaultControllerError");function si(e,t){const r=e._transformAlgorithm(t);return F(r,void 0,s=>{throw ii(e._controlledTransformStream,s),s})}n$1(si,"TransformStreamDefaultControllerPerformTransform");function Es(e){const t=e._controlledTransformStream,r=t._readable._readableStreamController;De(r);const s=new TypeError("TransformStream terminated");dn(t,s);}n$1(Es,"TransformStreamDefaultControllerTerminate");function As(e,t){const r=e._transformStreamController;if(e._backpressure){const s=e._backpressureChangePromise;return F(s,()=>{const u=e._writable;if(u._state==="erroring")throw u._storedError;return si(r,t)})}return si(r,t)}n$1(As,"TransformStreamDefaultSinkWriteAlgorithm");function Bs(e,t){const r=e._transformStreamController;if(r._finishPromise!==void 0)return r._finishPromise;const s=e._readable;r._finishPromise=A((c,d)=>{r._finishPromise_resolve=c,r._finishPromise_reject=d;});const u=r._cancelAlgorithm(t);return ur(r),g(u,()=>(s._state==="errored"?rt(r,s._storedError):(oe(s._readableStreamController,t),pn(r)),null),c=>(oe(s._readableStreamController,c),rt(r,c),null)),r._finishPromise}n$1(Bs,"TransformStreamDefaultSinkAbortAlgorithm");function ks(e){const t=e._transformStreamController;if(t._finishPromise!==void 0)return t._finishPromise;const r=e._readable;t._finishPromise=A((u,c)=>{t._finishPromise_resolve=u,t._finishPromise_reject=c;});const s=t._flushAlgorithm();return ur(t),g(s,()=>(r._state==="errored"?rt(t,r._storedError):(De(r._readableStreamController),pn(t)),null),u=>(oe(r._readableStreamController,u),rt(t,u),null)),t._finishPromise}n$1(ks,"TransformStreamDefaultSinkCloseAlgorithm");function Ws(e){return sr(e,false),e._backpressureChangePromise}n$1(Ws,"TransformStreamDefaultSourcePullAlgorithm");function qs(e,t){const r=e._transformStreamController;if(r._finishPromise!==void 0)return r._finishPromise;const s=e._writable;r._finishPromise=A((c,d)=>{r._finishPromise_resolve=c,r._finishPromise_reject=d;});const u=r._cancelAlgorithm(t);return ur(r),g(u,()=>(s._state==="errored"?rt(r,s._storedError):(Ct(s._writableStreamController,t),hn(e),pn(r)),null),c=>(Ct(s._writableStreamController,c),hn(e),rt(r,c),null)),r._finishPromise}n$1(qs,"TransformStreamDefaultSourceCancelAlgorithm");function fr(e){return new TypeError(`TransformStreamDefaultController.prototype.${e} can only be used on a TransformStreamDefaultController`)}n$1(fr,"defaultControllerBrandCheckException");function pn(e){e._finishPromise_resolve!==void 0&&(e._finishPromise_resolve(),e._finishPromise_resolve=void 0,e._finishPromise_reject=void 0);}n$1(pn,"defaultControllerFinishPromiseResolve");function rt(e,t){e._finishPromise_reject!==void 0&&(Q(e._finishPromise),e._finishPromise_reject(t),e._finishPromise_resolve=void 0,e._finishPromise_reject=void 0);}n$1(rt,"defaultControllerFinishPromiseReject");function li(e){return new TypeError(`TransformStream.prototype.${e} can only be used on a TransformStream`)}n$1(li,"streamBrandCheckException"),a.ByteLengthQueuingStrategy=Xe,a.CountQueuingStrategy=et,a.ReadableByteStreamController=te,a.ReadableStream=L,a.ReadableStreamBYOBReader=ce,a.ReadableStreamBYOBRequest=Re,a.ReadableStreamDefaultController=ne,a.ReadableStreamDefaultReader=fe,a.TransformStream=tt,a.TransformStreamDefaultController=pe,a.WritableStream=de,a.WritableStreamDefaultController=ke,a.WritableStreamDefaultWriter=re;});}(kt,kt.exports)),kt.exports}n$1(Ns,"requirePonyfill_es2018");var mi;function Hs(){if(mi)return pi;mi=1;const i=65536;if(!globalThis.ReadableStream)try{const o=require("node:process"),{emitWarning:a}=o;try{o.emitWarning=()=>{},Object.assign(globalThis,require("node:stream/web")),o.emitWarning=a;}catch(f){throw o.emitWarning=a,f}}catch{Object.assign(globalThis,Ns());}try{const{Blob:o}=require("buffer");o&&!o.prototype.stream&&(o.prototype.stream=n$1(function(f){let l=0;const p=this;return new ReadableStream({type:"bytes",async pull(h){const v=await p.slice(l,Math.min(p.size,l+i)).arrayBuffer();l+=v.byteLength,h.enqueue(new Uint8Array(v)),l===p.size&&h.close();}})},"name"));}catch{}return pi}n$1(Hs,"requireStreams"),Hs();/*! fetch-blob. MIT License. Jimmy Wärting <https://jimmy.warting.se/opensource> */const yi=65536;async function*Wn(i,o=true){for(const a of i)if("stream"in a)yield*a.stream();else if(ArrayBuffer.isView(a))if(o){let f=a.byteOffset;const l=a.byteOffset+a.byteLength;for(;f!==l;){const p=Math.min(l-f,yi),h=a.buffer.slice(f,f+p);f+=h.byteLength,yield new Uint8Array(h);}}else yield a;else {let f=0,l=a;for(;f!==l.size;){const h=await l.slice(f,Math.min(l.size,f+yi)).arrayBuffer();f+=h.byteLength,yield new Uint8Array(h);}}}n$1(Wn,"toIterator");const gi=(ze=class{constructor(o=[],a={}){be(this,ve,[]);be(this,zt,"");be(this,bt,0);be(this,Cr,"transparent");if(typeof o!="object"||o===null)throw new TypeError("Failed to construct 'Blob': The provided value cannot be converted to a sequence.");if(typeof o[Symbol.iterator]!="function")throw new TypeError("Failed to construct 'Blob': The object must have a callable @@iterator property.");if(typeof a!="object"&&typeof a!="function")throw new TypeError("Failed to construct 'Blob': parameter 2 cannot convert to dictionary.");a===null&&(a={});const f=new TextEncoder;for(const p of o){let h;ArrayBuffer.isView(p)?h=new Uint8Array(p.buffer.slice(p.byteOffset,p.byteOffset+p.byteLength)):p instanceof ArrayBuffer?h=new Uint8Array(p.slice(0)):p instanceof ze?h=p:h=f.encode(`${p}`),X(this,bt,O(this,bt)+(ArrayBuffer.isView(h)?h.byteLength:h.size)),O(this,ve).push(h);}X(this,Cr,`${a.endings===void 0?"transparent":a.endings}`);const l=a.type===void 0?"":String(a.type);X(this,zt,/^[\x20-\x7E]*$/.test(l)?l:"");}get size(){return O(this,bt)}get type(){return O(this,zt)}async text(){const o=new TextDecoder;let a="";for await(const f of Wn(O(this,ve),false))a+=o.decode(f,{stream:true});return a+=o.decode(),a}async arrayBuffer(){const o=new Uint8Array(this.size);let a=0;for await(const f of Wn(O(this,ve),false))o.set(f,a),a+=f.length;return o.buffer}stream(){const o=Wn(O(this,ve),true);return new globalThis.ReadableStream({type:"bytes",async pull(a){const f=await o.next();f.done?a.close():a.enqueue(f.value);},async cancel(){await o.return();}})}slice(o=0,a=this.size,f=""){const{size:l}=this;let p=o<0?Math.max(l+o,0):Math.min(o,l),h=a<0?Math.max(l+a,0):Math.min(a,l);const S=Math.max(h-p,0),v=O(this,ve),w=[];let A=0;for(const b of v){if(A>=S)break;const q=ArrayBuffer.isView(b)?b.byteLength:b.size;if(p&&q<=p)p-=q,h-=q;else {let g;ArrayBuffer.isView(b)?(g=b.subarray(p,Math.min(q,h)),A+=g.byteLength):(g=b.slice(p,Math.min(q,h)),A+=g.size),h-=q,w.push(g),p=0;}}const T=new ze([],{type:String(f).toLowerCase()});return X(T,bt,S),X(T,ve,w),T}get[Symbol.toStringTag](){return "Blob"}static[Symbol.hasInstance](o){return o&&typeof o=="object"&&typeof o.constructor=="function"&&(typeof o.stream=="function"||typeof o.arrayBuffer=="function")&&/^(Blob|File)$/.test(o[Symbol.toStringTag])}},ve=new WeakMap,zt=new WeakMap,bt=new WeakMap,Cr=new WeakMap,n$1(ze,"Blob"),ze);Object.defineProperties(gi.prototype,{size:{enumerable:true},type:{enumerable:true},slice:{enumerable:true}});const ut=gi,Vs=(mt=class extends ut{constructor(a,f,l={}){if(arguments.length<2)throw new TypeError(`Failed to construct 'File': 2 arguments required, but only ${arguments.length} present.`);super(a,l);be(this,It,0);be(this,Ft,"");l===null&&(l={});const p=l.lastModified===void 0?Date.now():Number(l.lastModified);Number.isNaN(p)||X(this,It,p),X(this,Ft,String(f));}get name(){return O(this,Ft)}get lastModified(){return O(this,It)}get[Symbol.toStringTag](){return "File"}static[Symbol.hasInstance](a){return !!a&&a instanceof ut&&/^(File)$/.test(a[Symbol.toStringTag])}},It=new WeakMap,Ft=new WeakMap,n$1(mt,"File"),mt),qn=Vs;/*! formdata-polyfill. MIT License. Jimmy Wärting <https://jimmy.warting.se/opensource> */var{toStringTag:Wt,iterator:Qs,hasInstance:Ys}=Symbol,_i=Math.random,Gs="append,set,get,getAll,delete,keys,values,entries,forEach,constructor".split(","),Si=n$1((i,o,a)=>(i+="",/^(Blob|File)$/.test(o&&o[Wt])?[(a=a!==void 0?a+"":o[Wt]=="File"?o.name:"blob",i),o.name!==a||o[Wt]=="blob"?new qn([o],a,o):o]:[i,o+""]),"f"),On=n$1((i,o)=>(o?i:i.replace(/\r?\n|\r/g,`\r
`)).replace(/\n/g,"%0A").replace(/\r/g,"%0D").replace(/"/g,"%22"),"e$1"),Ue=n$1((i,o,a)=>{if(o.length<a)throw new TypeError(`Failed to execute '${i}' on 'FormData': ${a} arguments required, but only ${o.length} present.`)},"x");const br=(yt=class{constructor(...o){be(this,ee,[]);if(o.length)throw new TypeError("Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.")}get[Wt](){return "FormData"}[Qs](){return this.entries()}static[Ys](o){return o&&typeof o=="object"&&o[Wt]==="FormData"&&!Gs.some(a=>typeof o[a]!="function")}append(...o){Ue("append",arguments,2),O(this,ee).push(Si(...o));}delete(o){Ue("delete",arguments,1),o+="",X(this,ee,O(this,ee).filter(([a])=>a!==o));}get(o){Ue("get",arguments,1),o+="";for(var a=O(this,ee),f=a.length,l=0;l<f;l++)if(a[l][0]===o)return a[l][1];return null}getAll(o,a){return Ue("getAll",arguments,1),a=[],o+="",O(this,ee).forEach(f=>f[0]===o&&a.push(f[1])),a}has(o){return Ue("has",arguments,1),o+="",O(this,ee).some(a=>a[0]===o)}forEach(o,a){Ue("forEach",arguments,1);for(var[f,l]of this)o.call(a,l,f,this);}set(...o){Ue("set",arguments,2);var a=[],f=true;o=Si(...o),O(this,ee).forEach(l=>{l[0]===o[0]?f&&(f=!a.push(o)):a.push(l);}),f&&a.push(o),X(this,ee,a);}*entries(){yield*O(this,ee);}*keys(){for(var[o]of this)yield o;}*values(){for(var[,o]of this)yield o;}},ee=new WeakMap,n$1(yt,"FormData"),yt);function Zs(i,o=ut){var a=`${_i()}${_i()}`.replace(/\./g,"").slice(-28).padStart(32,"-"),f=[],l=`--${a}\r
Content-Disposition: form-data; name="`;return i.forEach((p,h)=>typeof p=="string"?f.push(l+On(h)+`"\r
\r
${p.replace(/\r(?!\n)|(?<!\r)\n/g,`\r
`)}\r
`):f.push(l+On(h)+`"; filename="${On(p.name,1)}"\r
Content-Type: ${p.type||"application/octet-stream"}\r
\r
`,p,`\r
`)),f.push(`--${a}--`),new o(f,{type:"multipart/form-data; boundary="+a})}n$1(Zs,"formDataToBlob");const Un=class Un extends Error{constructor(o,a){super(o),Error.captureStackTrace(this,this.constructor),this.type=a;}get name(){return this.constructor.name}get[Symbol.toStringTag](){return this.constructor.name}};n$1(Un,"FetchBaseError");let ft=Un;const xn=class xn extends ft{constructor(o,a,f){super(o,a),f&&(this.code=this.errno=f.code,this.erroredSysCall=f.syscall);}};n$1(xn,"FetchError");let G=xn;const mr=Symbol.toStringTag,wi=n$1(i=>typeof i=="object"&&typeof i.append=="function"&&typeof i.delete=="function"&&typeof i.get=="function"&&typeof i.getAll=="function"&&typeof i.has=="function"&&typeof i.set=="function"&&typeof i.sort=="function"&&i[mr]==="URLSearchParams","isURLSearchParameters"),yr=n$1(i=>i&&typeof i=="object"&&typeof i.arrayBuffer=="function"&&typeof i.type=="string"&&typeof i.stream=="function"&&typeof i.constructor=="function"&&/^(Blob|File)$/.test(i[mr]),"isBlob"),Ks=n$1(i=>typeof i=="object"&&(i[mr]==="AbortSignal"||i[mr]==="EventTarget"),"isAbortSignal"),Js=n$1((i,o)=>{const a=new URL(o).hostname,f=new URL(i).hostname;return a===f||a.endsWith(`.${f}`)},"isDomainOrSubdomain"),Xs=n$1((i,o)=>{const a=new URL(o).protocol,f=new URL(i).protocol;return a===f},"isSameProtocol"),el=promisify(me.pipeline),H=Symbol("Body internals"),Nn=class Nn{constructor(o,{size:a=0}={}){let f=null;o===null?o=null:wi(o)?o=Buffer$1.from(o.toString()):yr(o)||Buffer$1.isBuffer(o)||(types.isAnyArrayBuffer(o)?o=Buffer$1.from(o):ArrayBuffer.isView(o)?o=Buffer$1.from(o.buffer,o.byteOffset,o.byteLength):o instanceof me||(o instanceof br?(o=Zs(o),f=o.type.split("=")[1]):o=Buffer$1.from(String(o))));let l=o;Buffer$1.isBuffer(o)?l=me.Readable.from(o):yr(o)&&(l=me.Readable.from(o.stream())),this[H]={body:o,stream:l,boundary:f,disturbed:false,error:null},this.size=a,o instanceof me&&o.on("error",p=>{const h=p instanceof ft?p:new G(`Invalid response body while trying to fetch ${this.url}: ${p.message}`,"system",p);this[H].error=h;});}get body(){return this[H].stream}get bodyUsed(){return this[H].disturbed}async arrayBuffer(){const{buffer:o,byteOffset:a,byteLength:f}=await zn(this);return o.slice(a,a+f)}async formData(){const o=this.headers.get("content-type");if(o.startsWith("application/x-www-form-urlencoded")){const f=new br,l=new URLSearchParams(await this.text());for(const[p,h]of l)f.append(p,h);return f}const{toFormData:a}=await import('./multipart-parser_B8fhMG2e.mjs');return a(this.body,o)}async blob(){const o=this.headers&&this.headers.get("content-type")||this[H].body&&this[H].body.type||"",a=await this.arrayBuffer();return new ut([a],{type:o})}async json(){const o=await this.text();return JSON.parse(o)}async text(){const o=await zn(this);return new TextDecoder().decode(o)}buffer(){return zn(this)}};n$1(Nn,"Body");let xe=Nn;xe.prototype.buffer=deprecate(xe.prototype.buffer,"Please use 'response.arrayBuffer()' instead of 'response.buffer()'","node-fetch#buffer"),Object.defineProperties(xe.prototype,{body:{enumerable:true},bodyUsed:{enumerable:true},arrayBuffer:{enumerable:true},blob:{enumerable:true},json:{enumerable:true},text:{enumerable:true},data:{get:deprecate(()=>{},"data doesn't exist, use json(), text(), arrayBuffer(), or body instead","https://github.com/node-fetch/node-fetch/issues/1000 (response)")}});async function zn(i){if(i[H].disturbed)throw new TypeError(`body used already for: ${i.url}`);if(i[H].disturbed=true,i[H].error)throw i[H].error;const{body:o}=i;if(o===null)return Buffer$1.alloc(0);if(!(o instanceof me))return Buffer$1.alloc(0);const a=[];let f=0;try{for await(const l of o){if(i.size>0&&f+l.length>i.size){const p=new G(`content size at ${i.url} over limit: ${i.size}`,"max-size");throw o.destroy(p),p}f+=l.length,a.push(l);}}catch(l){throw l instanceof ft?l:new G(`Invalid response body while trying to fetch ${i.url}: ${l.message}`,"system",l)}if(o.readableEnded===true||o._readableState.ended===true)try{return a.every(l=>typeof l=="string")?Buffer$1.from(a.join("")):Buffer$1.concat(a,f)}catch(l){throw new G(`Could not create Buffer from response body for ${i.url}: ${l.message}`,"system",l)}else throw new G(`Premature close of server response while trying to fetch ${i.url}`)}n$1(zn,"consumeBody");const In=n$1((i,o)=>{let a,f,{body:l}=i[H];if(i.bodyUsed)throw new Error("cannot clone body after it is used");return l instanceof me&&typeof l.getBoundary!="function"&&(a=new PassThrough({highWaterMark:o}),f=new PassThrough({highWaterMark:o}),l.pipe(a),l.pipe(f),i[H].stream=a,l=f),l},"clone"),tl=deprecate(i=>i.getBoundary(),"form-data doesn't follow the spec and requires special treatment. Use alternative package","https://github.com/node-fetch/node-fetch/issues/1167"),Ri=n$1((i,o)=>i===null?null:typeof i=="string"?"text/plain;charset=UTF-8":wi(i)?"application/x-www-form-urlencoded;charset=UTF-8":yr(i)?i.type||null:Buffer$1.isBuffer(i)||types.isAnyArrayBuffer(i)||ArrayBuffer.isView(i)?null:i instanceof br?`multipart/form-data; boundary=${o[H].boundary}`:i&&typeof i.getBoundary=="function"?`multipart/form-data;boundary=${tl(i)}`:i instanceof me?null:"text/plain;charset=UTF-8","extractContentType"),rl=n$1(i=>{const{body:o}=i[H];return o===null?0:yr(o)?o.size:Buffer$1.isBuffer(o)?o.length:o&&typeof o.getLengthSync=="function"&&o.hasKnownLength&&o.hasKnownLength()?o.getLengthSync():null},"getTotalBytes"),nl=n$1(async(i,{body:o})=>{o===null?i.end():await el(o,i);},"writeToStream"),gr=typeof Bt.validateHeaderName=="function"?Bt.validateHeaderName:i=>{if(!/^[\^`\-\w!#$%&'*+.|~]+$/.test(i)){const o=new TypeError(`Header name must be a valid HTTP token [${i}]`);throw Object.defineProperty(o,"code",{value:"ERR_INVALID_HTTP_TOKEN"}),o}},Fn=typeof Bt.validateHeaderValue=="function"?Bt.validateHeaderValue:(i,o)=>{if(/[^\t\u0020-\u007E\u0080-\u00FF]/.test(o)){const a=new TypeError(`Invalid character in header content ["${i}"]`);throw Object.defineProperty(a,"code",{value:"ERR_INVALID_CHAR"}),a}},Pr=class Pr extends URLSearchParams{constructor(o){let a=[];if(o instanceof Pr){const f=o.raw();for(const[l,p]of Object.entries(f))a.push(...p.map(h=>[l,h]));}else if(o!=null)if(typeof o=="object"&&!types.isBoxedPrimitive(o)){const f=o[Symbol.iterator];if(f==null)a.push(...Object.entries(o));else {if(typeof f!="function")throw new TypeError("Header pairs must be iterable");a=[...o].map(l=>{if(typeof l!="object"||types.isBoxedPrimitive(l))throw new TypeError("Each header pair must be an iterable object");return [...l]}).map(l=>{if(l.length!==2)throw new TypeError("Each header pair must be a name/value tuple");return [...l]});}}else throw new TypeError("Failed to construct 'Headers': The provided value is not of type '(sequence<sequence<ByteString>> or record<ByteString, ByteString>)");return a=a.length>0?a.map(([f,l])=>(gr(f),Fn(f,String(l)),[String(f).toLowerCase(),String(l)])):void 0,super(a),new Proxy(this,{get(f,l,p){switch(l){case "append":case "set":return (h,S)=>(gr(h),Fn(h,String(S)),URLSearchParams.prototype[l].call(f,String(h).toLowerCase(),String(S)));case "delete":case "has":case "getAll":return h=>(gr(h),URLSearchParams.prototype[l].call(f,String(h).toLowerCase()));case "keys":return ()=>(f.sort(),new Set(URLSearchParams.prototype.keys.call(f)).keys());default:return Reflect.get(f,l,p)}}})}get[Symbol.toStringTag](){return this.constructor.name}toString(){return Object.prototype.toString.call(this)}get(o){const a=this.getAll(o);if(a.length===0)return null;let f=a.join(", ");return /^content-encoding$/i.test(o)&&(f=f.toLowerCase()),f}forEach(o,a=void 0){for(const f of this.keys())Reflect.apply(o,a,[this.get(f),f,this]);}*values(){for(const o of this.keys())yield this.get(o);}*entries(){for(const o of this.keys())yield [o,this.get(o)];}[Symbol.iterator](){return this.entries()}raw(){return [...this.keys()].reduce((o,a)=>(o[a]=this.getAll(a),o),{})}[Symbol.for("nodejs.util.inspect.custom")](){return [...this.keys()].reduce((o,a)=>{const f=this.getAll(a);return a==="host"?o[a]=f[0]:o[a]=f.length>1?f:f[0],o},{})}};n$1(Pr,"Headers");let ye=Pr;Object.defineProperties(ye.prototype,["get","entries","forEach","values"].reduce((i,o)=>(i[o]={enumerable:true},i),{}));function ol(i=[]){return new ye(i.reduce((o,a,f,l)=>(f%2===0&&o.push(l.slice(f,f+2)),o),[]).filter(([o,a])=>{try{return gr(o),Fn(o,String(a)),!0}catch{return  false}}))}n$1(ol,"fromRawHeaders");const il=new Set([301,302,303,307,308]),jn=n$1(i=>il.has(i),"isRedirect"),se=Symbol("Response internals"),Ne=class Ne extends xe{constructor(o=null,a={}){super(o,a);const f=a.status!=null?a.status:200,l=new ye(a.headers);if(o!==null&&!l.has("Content-Type")){const p=Ri(o,this);p&&l.append("Content-Type",p);}this[se]={type:"default",url:a.url,status:f,statusText:a.statusText||"",headers:l,counter:a.counter,highWaterMark:a.highWaterMark};}get type(){return this[se].type}get url(){return this[se].url||""}get status(){return this[se].status}get ok(){return this[se].status>=200&&this[se].status<300}get redirected(){return this[se].counter>0}get statusText(){return this[se].statusText}get headers(){return this[se].headers}get highWaterMark(){return this[se].highWaterMark}clone(){return new Ne(In(this,this.highWaterMark),{type:this.type,url:this.url,status:this.status,statusText:this.statusText,headers:this.headers,ok:this.ok,redirected:this.redirected,size:this.size,highWaterMark:this.highWaterMark})}static redirect(o,a=302){if(!jn(a))throw new RangeError('Failed to execute "redirect" on "response": Invalid status code');return new Ne(null,{headers:{location:new URL(o).toString()},status:a})}static error(){const o=new Ne(null,{status:0,statusText:""});return o[se].type="error",o}static json(o=void 0,a={}){const f=JSON.stringify(o);if(f===void 0)throw new TypeError("data is not JSON serializable");const l=new ye(a&&a.headers);return l.has("content-type")||l.set("content-type","application/json"),new Ne(f,{...a,headers:l})}get[Symbol.toStringTag](){return "Response"}};n$1(Ne,"Response");let le=Ne;Object.defineProperties(le.prototype,{type:{enumerable:true},url:{enumerable:true},status:{enumerable:true},ok:{enumerable:true},redirected:{enumerable:true},statusText:{enumerable:true},headers:{enumerable:true},clone:{enumerable:true}});const al=n$1(i=>{if(i.search)return i.search;const o=i.href.length-1,a=i.hash||(i.href[o]==="#"?"#":"");return i.href[o-a.length]==="?"?"?":""},"getSearch");function Ti(i,o=false){return i==null||(i=new URL(i),/^(about|blob|data):$/.test(i.protocol))?"no-referrer":(i.username="",i.password="",i.hash="",o&&(i.pathname="",i.search=""),i)}n$1(Ti,"stripURLForUseAsAReferrer");const Ci=new Set(["","no-referrer","no-referrer-when-downgrade","same-origin","origin","strict-origin","origin-when-cross-origin","strict-origin-when-cross-origin","unsafe-url"]),sl="strict-origin-when-cross-origin";function ll(i){if(!Ci.has(i))throw new TypeError(`Invalid referrerPolicy: ${i}`);return i}n$1(ll,"validateReferrerPolicy");function ul(i){if(/^(http|ws)s:$/.test(i.protocol))return  true;const o=i.host.replace(/(^\[)|(]$)/g,""),a=isIP(o);return a===4&&/^127\./.test(o)||a===6&&/^(((0+:){7})|(::(0+:){0,6}))0*1$/.test(o)?true:i.host==="localhost"||i.host.endsWith(".localhost")?false:i.protocol==="file:"}n$1(ul,"isOriginPotentiallyTrustworthy");function ct(i){return /^about:(blank|srcdoc)$/.test(i)||i.protocol==="data:"||/^(blob|filesystem):$/.test(i.protocol)?true:ul(i)}n$1(ct,"isUrlPotentiallyTrustworthy");function fl(i,{referrerURLCallback:o,referrerOriginCallback:a}={}){if(i.referrer==="no-referrer"||i.referrerPolicy==="")return null;const f=i.referrerPolicy;if(i.referrer==="about:client")return "no-referrer";const l=i.referrer;let p=Ti(l),h=Ti(l,true);p.toString().length>4096&&(p=h),o&&(p=o(p)),a&&(h=a(h));const S=new URL(i.url);switch(f){case "no-referrer":return "no-referrer";case "origin":return h;case "unsafe-url":return p;case "strict-origin":return ct(p)&&!ct(S)?"no-referrer":h.toString();case "strict-origin-when-cross-origin":return p.origin===S.origin?p:ct(p)&&!ct(S)?"no-referrer":h;case "same-origin":return p.origin===S.origin?p:"no-referrer";case "origin-when-cross-origin":return p.origin===S.origin?p:h;case "no-referrer-when-downgrade":return ct(p)&&!ct(S)?"no-referrer":p;default:throw new TypeError(`Invalid referrerPolicy: ${f}`)}}n$1(fl,"determineRequestsReferrer");function cl(i){const o=(i.get("referrer-policy")||"").split(/[,\s]+/);let a="";for(const f of o)f&&Ci.has(f)&&(a=f);return a}n$1(cl,"parseReferrerPolicyFromHeader");const $=Symbol("Request internals"),qt=n$1(i=>typeof i=="object"&&typeof i[$]=="object","isRequest"),dl=deprecate(()=>{},".data is not a valid RequestInit property, use .body instead","https://github.com/node-fetch/node-fetch/issues/1000 (request)"),vr=class vr extends xe{constructor(o,a={}){let f;if(qt(o)?f=new URL(o.url):(f=new URL(o),o={}),f.username!==""||f.password!=="")throw new TypeError(`${f} is an url with embedded credentials.`);let l=a.method||o.method||"GET";if(/^(delete|get|head|options|post|put)$/i.test(l)&&(l=l.toUpperCase()),!qt(a)&&"data"in a&&dl(),(a.body!=null||qt(o)&&o.body!==null)&&(l==="GET"||l==="HEAD"))throw new TypeError("Request with GET/HEAD method cannot have body");const p=a.body?a.body:qt(o)&&o.body!==null?In(o):null;super(p,{size:a.size||o.size||0});const h=new ye(a.headers||o.headers||{});if(p!==null&&!h.has("Content-Type")){const w=Ri(p,this);w&&h.set("Content-Type",w);}let S=qt(o)?o.signal:null;if("signal"in a&&(S=a.signal),S!=null&&!Ks(S))throw new TypeError("Expected signal to be an instanceof AbortSignal or EventTarget");let v=a.referrer==null?o.referrer:a.referrer;if(v==="")v="no-referrer";else if(v){const w=new URL(v);v=/^about:(\/\/)?client$/.test(w)?"client":w;}else v=void 0;this[$]={method:l,redirect:a.redirect||o.redirect||"follow",headers:h,parsedURL:f,signal:S,referrer:v},this.follow=a.follow===void 0?o.follow===void 0?20:o.follow:a.follow,this.compress=a.compress===void 0?o.compress===void 0?true:o.compress:a.compress,this.counter=a.counter||o.counter||0,this.agent=a.agent||o.agent,this.highWaterMark=a.highWaterMark||o.highWaterMark||16384,this.insecureHTTPParser=a.insecureHTTPParser||o.insecureHTTPParser||false,this.referrerPolicy=a.referrerPolicy||o.referrerPolicy||"";}get method(){return this[$].method}get url(){return format(this[$].parsedURL)}get headers(){return this[$].headers}get redirect(){return this[$].redirect}get signal(){return this[$].signal}get referrer(){if(this[$].referrer==="no-referrer")return "";if(this[$].referrer==="client")return "about:client";if(this[$].referrer)return this[$].referrer.toString()}get referrerPolicy(){return this[$].referrerPolicy}set referrerPolicy(o){this[$].referrerPolicy=ll(o);}clone(){return new vr(this)}get[Symbol.toStringTag](){return "Request"}};n$1(vr,"Request");let dt=vr;Object.defineProperties(dt.prototype,{method:{enumerable:true},url:{enumerable:true},headers:{enumerable:true},redirect:{enumerable:true},clone:{enumerable:true},signal:{enumerable:true},referrer:{enumerable:true},referrerPolicy:{enumerable:true}});const hl=n$1(i=>{const{parsedURL:o}=i[$],a=new ye(i[$].headers);a.has("Accept")||a.set("Accept","*/*");let f=null;if(i.body===null&&/^(post|put)$/i.test(i.method)&&(f="0"),i.body!==null){const S=rl(i);typeof S=="number"&&!Number.isNaN(S)&&(f=String(S));}f&&a.set("Content-Length",f),i.referrerPolicy===""&&(i.referrerPolicy=sl),i.referrer&&i.referrer!=="no-referrer"?i[$].referrer=fl(i):i[$].referrer="no-referrer",i[$].referrer instanceof URL&&a.set("Referer",i.referrer),a.has("User-Agent")||a.set("User-Agent","node-fetch"),i.compress&&!a.has("Accept-Encoding")&&a.set("Accept-Encoding","gzip, deflate, br");let{agent:l}=i;typeof l=="function"&&(l=l(o));const p=al(o),h={path:o.pathname+p,method:i.method,headers:a[Symbol.for("nodejs.util.inspect.custom")](),insecureHTTPParser:i.insecureHTTPParser,agent:l};return {parsedURL:o,options:h}},"getNodeRequestOptions"),Hn=class Hn extends ft{constructor(o,a="aborted"){super(o,a);}};n$1(Hn,"AbortError");let _r=Hn;/*! node-domexception. MIT License. Jimmy Wärting <https://jimmy.warting.se/opensource> */var Ln,Pi;function pl(){if(Pi)return Ln;if(Pi=1,!globalThis.DOMException)try{const{MessageChannel:i}=require("worker_threads"),o=new i().port1,a=new ArrayBuffer;o.postMessage(a,[a,a]);}catch(i){i.constructor.name==="DOMException"&&(globalThis.DOMException=i.constructor);}return Ln=globalThis.DOMException,Ln}n$1(pl,"requireNodeDomexception");var bl=pl();const ml=f(bl),{stat:$n}=promises;n$1((i,o)=>vi(statSync(i),i,o),"blobFromSync");n$1((i,o)=>$n(i).then(a=>vi(a,i,o)),"blobFrom");n$1((i,o)=>$n(i).then(a=>Ei(a,i,o)),"fileFrom");n$1((i,o)=>Ei(statSync(i),i,o),"fileFromSync");const vi=n$1((i,o,a="")=>new ut([new Sr({path:o,size:i.size,lastModified:i.mtimeMs,start:0})],{type:a}),"fromBlob"),Ei=n$1((i,o,a="")=>new qn([new Sr({path:o,size:i.size,lastModified:i.mtimeMs,start:0})],basename(o),{type:a,lastModified:i.mtimeMs}),"fromFile"),Er=class Er{constructor(o){be(this,He);be(this,Ve);X(this,He,o.path),X(this,Ve,o.start),this.size=o.size,this.lastModified=o.lastModified;}slice(o,a){return new Er({path:O(this,He),lastModified:this.lastModified,size:a-o,start:O(this,Ve)+o})}async*stream(){const{mtimeMs:o}=await $n(O(this,He));if(o>this.lastModified)throw new ml("The requested file could not be read, typically due to permission problems that have occurred after a reference to a file was acquired.","NotReadableError");yield*createReadStream(O(this,He),{start:O(this,Ve),end:O(this,Ve)+this.size-1});}get[Symbol.toStringTag](){return "Blob"}};He=new WeakMap,Ve=new WeakMap,n$1(Er,"BlobDataItem");let Sr=Er;const wl=new Set(["data:","http:","https:"]);async function Ai(i,o){return new Promise((a,f)=>{const l=new dt(i,o),{parsedURL:p,options:h}=hl(l);if(!wl.has(p.protocol))throw new TypeError(`node-fetch cannot load ${i}. URL scheme "${p.protocol.replace(/:$/,"")}" is not supported.`);if(p.protocol==="data:"){const g=Us(l.url),V=new le(g,{headers:{"Content-Type":g.typeFull}});a(V);return}const S=(p.protocol==="https:"?zs:Bt).request,{signal:v}=l;let w=null;const A=n$1(()=>{const g=new _r("The operation was aborted.");f(g),l.body&&l.body instanceof me.Readable&&l.body.destroy(g),!(!w||!w.body)&&w.body.emit("error",g);},"abort");if(v&&v.aborted){A();return}const T=n$1(()=>{A(),q();},"abortAndFinalize"),b=S(p.toString(),h);v&&v.addEventListener("abort",T);const q=n$1(()=>{b.abort(),v&&v.removeEventListener("abort",T);},"finalize");b.on("error",g=>{f(new G(`request to ${l.url} failed, reason: ${g.message}`,"system",g)),q();}),Rl(b,g=>{w&&w.body&&w.body.destroy(g);}),process.version<"v14"&&b.on("socket",g=>{let V;g.prependListener("end",()=>{V=g._eventsCount;}),g.prependListener("close",I=>{if(w&&V<g._eventsCount&&!I){const F=new Error("Premature close");F.code="ERR_STREAM_PREMATURE_CLOSE",w.body.emit("error",F);}});}),b.on("response",g=>{b.setTimeout(0);const V=ol(g.rawHeaders);if(jn(g.statusCode)){const z=V.get("Location");let j=null;try{j=z===null?null:new URL(z,l.url);}catch{if(l.redirect!=="manual"){f(new G(`uri requested responds with an invalid redirect URL: ${z}`,"invalid-redirect")),q();return}}switch(l.redirect){case "error":f(new G(`uri requested responds with a redirect, redirect mode is set to error: ${l.url}`,"no-redirect")),q();return;case "manual":break;case "follow":{if(j===null)break;if(l.counter>=l.follow){f(new G(`maximum redirect reached at: ${l.url}`,"max-redirect")),q();return}const U={headers:new ye(l.headers),follow:l.follow,counter:l.counter+1,agent:l.agent,compress:l.compress,method:l.method,body:In(l),signal:l.signal,size:l.size,referrer:l.referrer,referrerPolicy:l.referrerPolicy};if(!Js(l.url,j)||!Xs(l.url,j))for(const jt of ["authorization","www-authenticate","cookie","cookie2"])U.headers.delete(jt);if(g.statusCode!==303&&l.body&&o.body instanceof me.Readable){f(new G("Cannot follow redirect with body being a readable stream","unsupported-redirect")),q();return}(g.statusCode===303||(g.statusCode===301||g.statusCode===302)&&l.method==="POST")&&(U.method="GET",U.body=void 0,U.headers.delete("content-length"));const D=cl(V);D&&(U.referrerPolicy=D),a(Ai(new dt(j,U))),q();return}default:return f(new TypeError(`Redirect option '${l.redirect}' is not a valid value of RequestRedirect`))}}v&&g.once("end",()=>{v.removeEventListener("abort",T);});let I=pipeline(g,new PassThrough,z=>{z&&f(z);});process.version<"v12.10"&&g.on("aborted",T);const F={url:l.url,status:g.statusCode,statusText:g.statusMessage,headers:V,size:l.size,counter:l.counter,highWaterMark:l.highWaterMark},Q=V.get("Content-Encoding");if(!l.compress||l.method==="HEAD"||Q===null||g.statusCode===204||g.statusCode===304){w=new le(I,F),a(w);return}const ge={flush:st.Z_SYNC_FLUSH,finishFlush:st.Z_SYNC_FLUSH};if(Q==="gzip"||Q==="x-gzip"){I=pipeline(I,st.createGunzip(ge),z=>{z&&f(z);}),w=new le(I,F),a(w);return}if(Q==="deflate"||Q==="x-deflate"){const z=pipeline(g,new PassThrough,j=>{j&&f(j);});z.once("data",j=>{(j[0]&15)===8?I=pipeline(I,st.createInflate(),U=>{U&&f(U);}):I=pipeline(I,st.createInflateRaw(),U=>{U&&f(U);}),w=new le(I,F),a(w);}),z.once("end",()=>{w||(w=new le(I,F),a(w));});return}if(Q==="br"){I=pipeline(I,st.createBrotliDecompress(),z=>{z&&f(z);}),w=new le(I,F),a(w);return}w=new le(I,F),a(w);}),nl(b,l).catch(f);})}n$1(Ai,"fetch$1");function Rl(i,o){const a=Buffer$1.from(`0\r
\r
`);let f=false,l=false,p;i.on("response",h=>{const{headers:S}=h;f=S["transfer-encoding"]==="chunked"&&!S["content-length"];}),i.on("socket",h=>{const S=n$1(()=>{if(f&&!l){const w=new Error("Premature close");w.code="ERR_STREAM_PREMATURE_CLOSE",o(w);}},"onSocketClose"),v=n$1(w=>{l=Buffer$1.compare(w.slice(-5),a)===0,!l&&p&&(l=Buffer$1.compare(p.slice(-3),a.slice(0,3))===0&&Buffer$1.compare(w.slice(-2),a.slice(3))===0),p=w;},"onData");h.prependListener("close",S),h.on("data",v),i.on("close",()=>{h.removeListener("close",S),h.removeListener("data",v);});});}n$1(Rl,"fixResponseChunkedTransferBadEnding");const Bi=new WeakMap,Dn=new WeakMap;function W(i){const o=Bi.get(i);return console.assert(o!=null,"'this' is expected an Event object, but got",i),o}n$1(W,"pd");function ki(i){if(i.passiveListener!=null){typeof console<"u"&&typeof console.error=="function"&&console.error("Unable to preventDefault inside passive event listener invocation.",i.passiveListener);return}i.event.cancelable&&(i.canceled=true,typeof i.event.preventDefault=="function"&&i.event.preventDefault());}n$1(ki,"setCancelFlag");function ht(i,o){Bi.set(this,{eventTarget:i,event:o,eventPhase:2,currentTarget:i,canceled:false,stopped:false,immediateStopped:false,passiveListener:null,timeStamp:o.timeStamp||Date.now()}),Object.defineProperty(this,"isTrusted",{value:false,enumerable:true});const a=Object.keys(o);for(let f=0;f<a.length;++f){const l=a[f];l in this||Object.defineProperty(this,l,Wi(l));}}n$1(ht,"Event"),ht.prototype={get type(){return W(this).event.type},get target(){return W(this).eventTarget},get currentTarget(){return W(this).currentTarget},composedPath(){const i=W(this).currentTarget;return i==null?[]:[i]},get NONE(){return 0},get CAPTURING_PHASE(){return 1},get AT_TARGET(){return 2},get BUBBLING_PHASE(){return 3},get eventPhase(){return W(this).eventPhase},stopPropagation(){const i=W(this);i.stopped=true,typeof i.event.stopPropagation=="function"&&i.event.stopPropagation();},stopImmediatePropagation(){const i=W(this);i.stopped=true,i.immediateStopped=true,typeof i.event.stopImmediatePropagation=="function"&&i.event.stopImmediatePropagation();},get bubbles(){return !!W(this).event.bubbles},get cancelable(){return !!W(this).event.cancelable},preventDefault(){ki(W(this));},get defaultPrevented(){return W(this).canceled},get composed(){return !!W(this).event.composed},get timeStamp(){return W(this).timeStamp},get srcElement(){return W(this).eventTarget},get cancelBubble(){return W(this).stopped},set cancelBubble(i){if(!i)return;const o=W(this);o.stopped=true,typeof o.event.cancelBubble=="boolean"&&(o.event.cancelBubble=true);},get returnValue(){return !W(this).canceled},set returnValue(i){i||ki(W(this));},initEvent(){}},Object.defineProperty(ht.prototype,"constructor",{value:ht,configurable:true,writable:true}),typeof window<"u"&&typeof window.Event<"u"&&(Object.setPrototypeOf(ht.prototype,window.Event.prototype),Dn.set(window.Event.prototype,ht));function Wi(i){return {get(){return W(this).event[i]},set(o){W(this).event[i]=o;},configurable:true,enumerable:true}}n$1(Wi,"defineRedirectDescriptor");function Tl(i){return {value(){const o=W(this).event;return o[i].apply(o,arguments)},configurable:true,enumerable:true}}n$1(Tl,"defineCallDescriptor");function Cl(i,o){const a=Object.keys(o);if(a.length===0)return i;function f(l,p){i.call(this,l,p);}n$1(f,"CustomEvent"),f.prototype=Object.create(i.prototype,{constructor:{value:f,configurable:true,writable:true}});for(let l=0;l<a.length;++l){const p=a[l];if(!(p in i.prototype)){const S=typeof Object.getOwnPropertyDescriptor(o,p).value=="function";Object.defineProperty(f.prototype,p,S?Tl(p):Wi(p));}}return f}n$1(Cl,"defineWrapper");function qi(i){if(i==null||i===Object.prototype)return ht;let o=Dn.get(i);return o==null&&(o=Cl(qi(Object.getPrototypeOf(i)),i),Dn.set(i,o)),o}n$1(qi,"getWrapper");function Pl(i,o){const a=qi(Object.getPrototypeOf(o));return new a(i,o)}n$1(Pl,"wrapEvent");function vl(i){return W(i).immediateStopped}n$1(vl,"isStopped");function El(i,o){W(i).eventPhase=o;}n$1(El,"setEventPhase");function Al(i,o){W(i).currentTarget=o;}n$1(Al,"setCurrentTarget");function Oi(i,o){W(i).passiveListener=o;}n$1(Oi,"setPassiveListener");const zi=new WeakMap,Ii=1,Fi=2,wr=3;function Rr(i){return i!==null&&typeof i=="object"}n$1(Rr,"isObject");function Ot(i){const o=zi.get(i);if(o==null)throw new TypeError("'this' is expected an EventTarget object, but got another value.");return o}n$1(Ot,"getListeners");function Bl(i){return {get(){let a=Ot(this).get(i);for(;a!=null;){if(a.listenerType===wr)return a.listener;a=a.next;}return null},set(o){typeof o!="function"&&!Rr(o)&&(o=null);const a=Ot(this);let f=null,l=a.get(i);for(;l!=null;)l.listenerType===wr?f!==null?f.next=l.next:l.next!==null?a.set(i,l.next):a.delete(i):f=l,l=l.next;if(o!==null){const p={listener:o,listenerType:wr,passive:false,once:false,next:null};f===null?a.set(i,p):f.next=p;}},configurable:true,enumerable:true}}n$1(Bl,"defineEventAttributeDescriptor");function ji(i,o){Object.defineProperty(i,`on${o}`,Bl(o));}n$1(ji,"defineEventAttribute");function Li(i){function o(){Pe.call(this);}n$1(o,"CustomEventTarget"),o.prototype=Object.create(Pe.prototype,{constructor:{value:o,configurable:true,writable:true}});for(let a=0;a<i.length;++a)ji(o.prototype,i[a]);return o}n$1(Li,"defineCustomEventTarget");function Pe(){if(this instanceof Pe){zi.set(this,new Map);return}if(arguments.length===1&&Array.isArray(arguments[0]))return Li(arguments[0]);if(arguments.length>0){const i=new Array(arguments.length);for(let o=0;o<arguments.length;++o)i[o]=arguments[o];return Li(i)}throw new TypeError("Cannot call a class as a function")}n$1(Pe,"EventTarget"),Pe.prototype={addEventListener(i,o,a){if(o==null)return;if(typeof o!="function"&&!Rr(o))throw new TypeError("'listener' should be a function or an object.");const f=Ot(this),l=Rr(a),h=(l?!!a.capture:!!a)?Ii:Fi,S={listener:o,listenerType:h,passive:l&&!!a.passive,once:l&&!!a.once,next:null};let v=f.get(i);if(v===void 0){f.set(i,S);return}let w=null;for(;v!=null;){if(v.listener===o&&v.listenerType===h)return;w=v,v=v.next;}w.next=S;},removeEventListener(i,o,a){if(o==null)return;const f=Ot(this),p=(Rr(a)?!!a.capture:!!a)?Ii:Fi;let h=null,S=f.get(i);for(;S!=null;){if(S.listener===o&&S.listenerType===p){h!==null?h.next=S.next:S.next!==null?f.set(i,S.next):f.delete(i);return}h=S,S=S.next;}},dispatchEvent(i){if(i==null||typeof i.type!="string")throw new TypeError('"event.type" should be a string.');const o=Ot(this),a=i.type;let f=o.get(a);if(f==null)return  true;const l=Pl(this,i);let p=null;for(;f!=null;){if(f.once?p!==null?p.next=f.next:f.next!==null?o.set(a,f.next):o.delete(a):p=f,Oi(l,f.passive?f.listener:null),typeof f.listener=="function")try{f.listener.call(this,l);}catch(h){typeof console<"u"&&typeof console.error=="function"&&console.error(h);}else f.listenerType!==wr&&typeof f.listener.handleEvent=="function"&&f.listener.handleEvent(l);if(vl(l))break;f=f.next;}return Oi(l,null),El(l,0),Al(l,null),!l.defaultPrevented}},Object.defineProperty(Pe.prototype,"constructor",{value:Pe,configurable:true,writable:true}),typeof window<"u"&&typeof window.EventTarget<"u"&&Object.setPrototypeOf(Pe.prototype,window.EventTarget.prototype);const Vn=class Vn extends Pe{constructor(){throw super(),new TypeError("AbortSignal cannot be constructed directly")}get aborted(){const o=Tr.get(this);if(typeof o!="boolean")throw new TypeError(`Expected 'this' to be an 'AbortSignal' object, but got ${this===null?"null":typeof this}`);return o}};n$1(Vn,"AbortSignal");let pt=Vn;ji(pt.prototype,"abort");function kl(){const i=Object.create(pt.prototype);return Pe.call(i),Tr.set(i,false),i}n$1(kl,"createAbortSignal");function Wl(i){Tr.get(i)===false&&(Tr.set(i,true),i.dispatchEvent({type:"abort"}));}n$1(Wl,"abortSignal");const Tr=new WeakMap;Object.defineProperties(pt.prototype,{aborted:{enumerable:true}}),typeof Symbol=="function"&&typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(pt.prototype,Symbol.toStringTag,{configurable:true,value:"AbortSignal"});let Mn=(gt=class{constructor(){$i.set(this,kl());}get signal(){return Di(this)}abort(){Wl(Di(this));}},n$1(gt,"AbortController"),gt);const $i=new WeakMap;function Di(i){const o=$i.get(i);if(o==null)throw new TypeError(`Expected 'this' to be an 'AbortController' object, but got ${i===null?"null":typeof i}`);return o}n$1(Di,"getSignal"),Object.defineProperties(Mn.prototype,{signal:{enumerable:true},abort:{enumerable:true}}),typeof Symbol=="function"&&typeof Symbol.toStringTag=="symbol"&&Object.defineProperty(Mn.prototype,Symbol.toStringTag,{configurable:true,value:"AbortController"});var ql=Object.defineProperty,Ol=n$1((i,o)=>ql(i,"name",{value:o,configurable:true}),"e");const Mi=Ai;Ui();function Ui(){!globalThis.process?.versions?.node&&!globalThis.process?.env?.DISABLE_NODE_FETCH_NATIVE_WARN&&console.warn("[node-fetch-native] Node.js compatible build of `node-fetch-native` is being used in a non-Node.js environment. Please make sure you are using proper export conditions or report this issue to https://github.com/unjs/node-fetch-native. You can set `process.env.DISABLE_NODE_FETCH_NATIVE_WARN` to disable this warning.");}n$1(Ui,"s"),Ol(Ui,"checkNodeEnvironment");

const o=!!globalThis.process?.env?.FORCE_NODE_FETCH,r=!o&&globalThis.fetch||Mi,n=!o&&globalThis.Headers||ye,T=!o&&globalThis.AbortController||Mn;

const HASH_RE = /#/g;
const AMPERSAND_RE = /&/g;
const SLASH_RE = /\//g;
const EQUAL_RE = /=/g;
const PLUS_RE = /\+/g;
const ENC_CARET_RE = /%5e/gi;
const ENC_BACKTICK_RE = /%60/gi;
const ENC_PIPE_RE = /%7c/gi;
const ENC_SPACE_RE = /%20/gi;
function encode(text) {
  return encodeURI("" + text).replace(ENC_PIPE_RE, "|");
}
function encodeQueryValue(input) {
  return encode(typeof input === "string" ? input : JSON.stringify(input)).replace(PLUS_RE, "%2B").replace(ENC_SPACE_RE, "+").replace(HASH_RE, "%23").replace(AMPERSAND_RE, "%26").replace(ENC_BACKTICK_RE, "`").replace(ENC_CARET_RE, "^").replace(SLASH_RE, "%2F");
}
function encodeQueryKey(text) {
  return encodeQueryValue(text).replace(EQUAL_RE, "%3D");
}
function decode(text = "") {
  try {
    return decodeURIComponent("" + text);
  } catch {
    return "" + text;
  }
}
function decodeQueryKey(text) {
  return decode(text.replace(PLUS_RE, " "));
}
function decodeQueryValue(text) {
  return decode(text.replace(PLUS_RE, " "));
}

function parseQuery(parametersString = "") {
  const object = /* @__PURE__ */ Object.create(null);
  if (parametersString[0] === "?") {
    parametersString = parametersString.slice(1);
  }
  for (const parameter of parametersString.split("&")) {
    const s = parameter.match(/([^=]+)=?(.*)/) || [];
    if (s.length < 2) {
      continue;
    }
    const key = decodeQueryKey(s[1]);
    if (key === "__proto__" || key === "constructor") {
      continue;
    }
    const value = decodeQueryValue(s[2] || "");
    if (object[key] === void 0) {
      object[key] = value;
    } else if (Array.isArray(object[key])) {
      object[key].push(value);
    } else {
      object[key] = [object[key], value];
    }
  }
  return object;
}
function encodeQueryItem(key, value) {
  if (typeof value === "number" || typeof value === "boolean") {
    value = String(value);
  }
  if (!value) {
    return encodeQueryKey(key);
  }
  if (Array.isArray(value)) {
    return value.map(
      (_value) => `${encodeQueryKey(key)}=${encodeQueryValue(_value)}`
    ).join("&");
  }
  return `${encodeQueryKey(key)}=${encodeQueryValue(value)}`;
}
function stringifyQuery(query) {
  return Object.keys(query).filter((k) => query[k] !== void 0).map((k) => encodeQueryItem(k, query[k])).filter(Boolean).join("&");
}

const PROTOCOL_STRICT_REGEX = /^[\s\w\0+.-]{2,}:([/\\]{1,2})/;
const PROTOCOL_REGEX = /^[\s\w\0+.-]{2,}:([/\\]{2})?/;
const PROTOCOL_RELATIVE_REGEX = /^([/\\]\s*){2,}[^/\\]/;
const JOIN_LEADING_SLASH_RE = /^\.?\//;
function hasProtocol(inputString, opts = {}) {
  if (typeof opts === "boolean") {
    opts = { acceptRelative: opts };
  }
  if (opts.strict) {
    return PROTOCOL_STRICT_REGEX.test(inputString);
  }
  return PROTOCOL_REGEX.test(inputString) || (opts.acceptRelative ? PROTOCOL_RELATIVE_REGEX.test(inputString) : false);
}
function hasTrailingSlash(input = "", respectQueryAndFragment) {
  {
    return input.endsWith("/");
  }
}
function withoutTrailingSlash(input = "", respectQueryAndFragment) {
  {
    return (hasTrailingSlash(input) ? input.slice(0, -1) : input) || "/";
  }
}
function withTrailingSlash(input = "", respectQueryAndFragment) {
  {
    return input.endsWith("/") ? input : input + "/";
  }
}
function withBase(input, base) {
  if (isEmptyURL(base) || hasProtocol(input)) {
    return input;
  }
  const _base = withoutTrailingSlash(base);
  if (input.startsWith(_base)) {
    const nextChar = input[_base.length];
    if (!nextChar || nextChar === "/" || nextChar === "?") {
      return input;
    }
  }
  return joinURL(_base, input);
}
function withQuery(input, query) {
  const parsed = parseURL(input);
  const mergedQuery = { ...parseQuery(parsed.search), ...query };
  parsed.search = stringifyQuery(mergedQuery);
  return stringifyParsedURL(parsed);
}
function isEmptyURL(url) {
  return !url || url === "/";
}
function isNonEmptyURL(url) {
  return url && url !== "/";
}
function joinURL(base, ...input) {
  let url = base || "";
  for (const segment of input.filter((url2) => isNonEmptyURL(url2))) {
    if (url) {
      const _segment = segment.replace(JOIN_LEADING_SLASH_RE, "");
      url = withTrailingSlash(url) + _segment;
    } else {
      url = segment;
    }
  }
  return url;
}

const protocolRelative = Symbol.for("ufo:protocolRelative");
function parseURL(input = "", defaultProto) {
  const _specialProtoMatch = input.match(
    /^[\s\0]*(blob:|data:|javascript:|vbscript:)(.*)/i
  );
  if (_specialProtoMatch) {
    const [, _proto, _pathname = ""] = _specialProtoMatch;
    return {
      protocol: _proto.toLowerCase(),
      pathname: _pathname,
      href: _proto + _pathname,
      auth: "",
      host: "",
      search: "",
      hash: ""
    };
  }
  if (!hasProtocol(input, { acceptRelative: true })) {
    return parsePath(input);
  }
  const [, protocol = "", auth, hostAndPath = ""] = input.replace(/\\/g, "/").match(/^[\s\0]*([\w+.-]{2,}:)?\/\/([^/@]+@)?(.*)/) || [];
  let [, host = "", path = ""] = hostAndPath.match(/([^#/?]*)(.*)?/) || [];
  if (protocol === "file:") {
    path = path.replace(/\/(?=[A-Za-z]:)/, "");
  }
  const { pathname, search, hash } = parsePath(path);
  return {
    protocol: protocol.toLowerCase(),
    auth: auth ? auth.slice(0, Math.max(0, auth.length - 1)) : "",
    host,
    pathname,
    search,
    hash,
    [protocolRelative]: !protocol
  };
}
function parsePath(input = "") {
  const [pathname = "", search = "", hash = ""] = (input.match(/([^#?]*)(\?[^#]*)?(#.*)?/) || []).splice(1);
  return {
    pathname,
    search,
    hash
  };
}
function stringifyParsedURL(parsed) {
  const pathname = parsed.pathname || "";
  const search = parsed.search ? (parsed.search.startsWith("?") ? "" : "?") + parsed.search : "";
  const hash = parsed.hash || "";
  const auth = parsed.auth ? parsed.auth + "@" : "";
  const host = parsed.host || "";
  const proto = parsed.protocol || parsed[protocolRelative] ? (parsed.protocol || "") + "//" : "";
  return proto + auth + host + pathname + search + hash;
}

class FetchError extends Error {
  constructor(message, opts) {
    super(message, opts);
    this.name = "FetchError";
    if (opts?.cause && !this.cause) {
      this.cause = opts.cause;
    }
  }
}
function createFetchError(ctx) {
  const errorMessage = ctx.error?.message || ctx.error?.toString() || "";
  const method = ctx.request?.method || ctx.options?.method || "GET";
  const url = ctx.request?.url || String(ctx.request) || "/";
  const requestStr = `[${method}] ${JSON.stringify(url)}`;
  const statusStr = ctx.response ? `${ctx.response.status} ${ctx.response.statusText}` : "<no response>";
  const message = `${requestStr}: ${statusStr}${errorMessage ? ` ${errorMessage}` : ""}`;
  const fetchError = new FetchError(
    message,
    ctx.error ? { cause: ctx.error } : void 0
  );
  for (const key of ["request", "options", "response"]) {
    Object.defineProperty(fetchError, key, {
      get() {
        return ctx[key];
      }
    });
  }
  for (const [key, refKey] of [
    ["data", "_data"],
    ["status", "status"],
    ["statusCode", "status"],
    ["statusText", "statusText"],
    ["statusMessage", "statusText"]
  ]) {
    Object.defineProperty(fetchError, key, {
      get() {
        return ctx.response && ctx.response[refKey];
      }
    });
  }
  return fetchError;
}

const payloadMethods = new Set(
  Object.freeze(["PATCH", "POST", "PUT", "DELETE"])
);
function isPayloadMethod(method = "GET") {
  return payloadMethods.has(method.toUpperCase());
}
function isJSONSerializable(value) {
  if (value === void 0) {
    return false;
  }
  const t = typeof value;
  if (t === "string" || t === "number" || t === "boolean" || t === null) {
    return true;
  }
  if (t !== "object") {
    return false;
  }
  if (Array.isArray(value)) {
    return true;
  }
  if (value.buffer) {
    return false;
  }
  if (value instanceof FormData || value instanceof URLSearchParams) {
    return false;
  }
  return value.constructor && value.constructor.name === "Object" || typeof value.toJSON === "function";
}
const textTypes = /* @__PURE__ */ new Set([
  "image/svg",
  "application/xml",
  "application/xhtml",
  "application/html"
]);
const JSON_RE = /^application\/(?:[\w!#$%&*.^`~-]*\+)?json(;.+)?$/i;
function detectResponseType(_contentType = "") {
  if (!_contentType) {
    return "json";
  }
  const contentType = _contentType.split(";").shift() || "";
  if (JSON_RE.test(contentType)) {
    return "json";
  }
  if (contentType === "text/event-stream") {
    return "stream";
  }
  if (textTypes.has(contentType) || contentType.startsWith("text/")) {
    return "text";
  }
  return "blob";
}
function resolveFetchOptions(request, input, defaults, Headers) {
  const headers = mergeHeaders(
    input?.headers ?? request?.headers,
    defaults?.headers,
    Headers
  );
  let query;
  if (defaults?.query || defaults?.params || input?.params || input?.query) {
    query = {
      ...defaults?.params,
      ...defaults?.query,
      ...input?.params,
      ...input?.query
    };
  }
  return {
    ...defaults,
    ...input,
    query,
    params: query,
    headers
  };
}
function mergeHeaders(input, defaults, Headers) {
  if (!defaults) {
    return new Headers(input);
  }
  const headers = new Headers(defaults);
  if (input) {
    for (const [key, value] of Symbol.iterator in input || Array.isArray(input) ? input : new Headers(input)) {
      headers.set(key, value);
    }
  }
  return headers;
}
async function callHooks(context, hooks) {
  if (hooks) {
    if (Array.isArray(hooks)) {
      for (const hook of hooks) {
        await hook(context);
      }
    } else {
      await hooks(context);
    }
  }
}

const retryStatusCodes = /* @__PURE__ */ new Set([
  408,
  // Request Timeout
  409,
  // Conflict
  425,
  // Too Early (Experimental)
  429,
  // Too Many Requests
  500,
  // Internal Server Error
  502,
  // Bad Gateway
  503,
  // Service Unavailable
  504
  // Gateway Timeout
]);
const nullBodyResponses = /* @__PURE__ */ new Set([101, 204, 205, 304]);
function createFetch(globalOptions = {}) {
  const {
    fetch = globalThis.fetch,
    Headers = globalThis.Headers,
    AbortController = globalThis.AbortController
  } = globalOptions;
  async function onError(context) {
    const isAbort = context.error && context.error.name === "AbortError" && !context.options.timeout || false;
    if (context.options.retry !== false && !isAbort) {
      let retries;
      if (typeof context.options.retry === "number") {
        retries = context.options.retry;
      } else {
        retries = isPayloadMethod(context.options.method) ? 0 : 1;
      }
      const responseCode = context.response && context.response.status || 500;
      if (retries > 0 && (Array.isArray(context.options.retryStatusCodes) ? context.options.retryStatusCodes.includes(responseCode) : retryStatusCodes.has(responseCode))) {
        const retryDelay = typeof context.options.retryDelay === "function" ? context.options.retryDelay(context) : context.options.retryDelay || 0;
        if (retryDelay > 0) {
          await new Promise((resolve) => setTimeout(resolve, retryDelay));
        }
        return $fetchRaw(context.request, {
          ...context.options,
          retry: retries - 1
        });
      }
    }
    const error = createFetchError(context);
    if (Error.captureStackTrace) {
      Error.captureStackTrace(error, $fetchRaw);
    }
    throw error;
  }
  const $fetchRaw = async function $fetchRaw2(_request, _options = {}) {
    const context = {
      request: _request,
      options: resolveFetchOptions(
        _request,
        _options,
        globalOptions.defaults,
        Headers
      ),
      response: void 0,
      error: void 0
    };
    if (context.options.method) {
      context.options.method = context.options.method.toUpperCase();
    }
    if (context.options.onRequest) {
      await callHooks(context, context.options.onRequest);
      if (!(context.options.headers instanceof Headers)) {
        context.options.headers = new Headers(
          context.options.headers || {}
          /* compat */
        );
      }
    }
    if (typeof context.request === "string") {
      if (context.options.baseURL) {
        context.request = withBase(context.request, context.options.baseURL);
      }
      if (context.options.query) {
        context.request = withQuery(context.request, context.options.query);
        delete context.options.query;
      }
      if ("query" in context.options) {
        delete context.options.query;
      }
      if ("params" in context.options) {
        delete context.options.params;
      }
    }
    if (context.options.body && isPayloadMethod(context.options.method)) {
      if (isJSONSerializable(context.options.body)) {
        const contentType = context.options.headers.get("content-type");
        if (typeof context.options.body !== "string") {
          context.options.body = contentType === "application/x-www-form-urlencoded" ? new URLSearchParams(
            context.options.body
          ).toString() : JSON.stringify(context.options.body);
        }
        if (!contentType) {
          context.options.headers.set("content-type", "application/json");
        }
        if (!context.options.headers.has("accept")) {
          context.options.headers.set("accept", "application/json");
        }
      } else if (
        // ReadableStream Body
        "pipeTo" in context.options.body && typeof context.options.body.pipeTo === "function" || // Node.js Stream Body
        typeof context.options.body.pipe === "function"
      ) {
        if (!("duplex" in context.options)) {
          context.options.duplex = "half";
        }
      }
    }
    let abortTimeout;
    if (!context.options.signal && context.options.timeout) {
      const controller = new AbortController();
      abortTimeout = setTimeout(() => {
        const error = new Error(
          "[TimeoutError]: The operation was aborted due to timeout"
        );
        error.name = "TimeoutError";
        error.code = 23;
        controller.abort(error);
      }, context.options.timeout);
      context.options.signal = controller.signal;
    }
    try {
      context.response = await fetch(
        context.request,
        context.options
      );
    } catch (error) {
      context.error = error;
      if (context.options.onRequestError) {
        await callHooks(
          context,
          context.options.onRequestError
        );
      }
      return await onError(context);
    } finally {
      if (abortTimeout) {
        clearTimeout(abortTimeout);
      }
    }
    const hasBody = (context.response.body || // https://github.com/unjs/ofetch/issues/324
    // https://github.com/unjs/ofetch/issues/294
    // https://github.com/JakeChampion/fetch/issues/1454
    context.response._bodyInit) && !nullBodyResponses.has(context.response.status) && context.options.method !== "HEAD";
    if (hasBody) {
      const responseType = (context.options.parseResponse ? "json" : context.options.responseType) || detectResponseType(context.response.headers.get("content-type") || "");
      switch (responseType) {
        case "json": {
          const data = await context.response.text();
          const parseFunction = context.options.parseResponse || destr;
          context.response._data = parseFunction(data);
          break;
        }
        case "stream": {
          context.response._data = context.response.body || context.response._bodyInit;
          break;
        }
        default: {
          context.response._data = await context.response[responseType]();
        }
      }
    }
    if (context.options.onResponse) {
      await callHooks(
        context,
        context.options.onResponse
      );
    }
    if (!context.options.ignoreResponseError && context.response.status >= 400 && context.response.status < 600) {
      if (context.options.onResponseError) {
        await callHooks(
          context,
          context.options.onResponseError
        );
      }
      return await onError(context);
    }
    return context.response;
  };
  const $fetch = async function $fetch2(request, options) {
    const r = await $fetchRaw(request, options);
    return r._data;
  };
  $fetch.raw = $fetchRaw;
  $fetch.native = (...args) => fetch(...args);
  $fetch.create = (defaultOptions = {}, customGlobalOptions = {}) => createFetch({
    ...globalOptions,
    ...customGlobalOptions,
    defaults: {
      ...globalOptions.defaults,
      ...customGlobalOptions.defaults,
      ...defaultOptions
    }
  });
  return $fetch;
}

function createNodeFetch() {
  const useKeepAlive = JSON.parse(process.env.FETCH_KEEP_ALIVE || "false");
  if (!useKeepAlive) {
    return r;
  }
  const agentOptions = { keepAlive: true };
  const httpAgent = new Bt.Agent(agentOptions);
  const httpsAgent = new zs.Agent(agentOptions);
  const nodeFetchOptions = {
    agent(parsedURL) {
      return parsedURL.protocol === "http:" ? httpAgent : httpsAgent;
    }
  };
  return function nodeFetchWithKeepAlive(input, init) {
    return r(input, { ...nodeFetchOptions, ...init });
  };
}
const fetch = globalThis.fetch ? (...args) => globalThis.fetch(...args) : createNodeFetch();
const Headers = globalThis.Headers || n;
const AbortController$1 = globalThis.AbortController || T;
createFetch({ fetch, Headers, AbortController: AbortController$1 });

const DRIVER_NAME = "netlify-blobs";
const netlifyBlobs = defineDriver((options) => {
  const { deployScoped, name, ...opts } = options;
  let store;
  const getClient = () => {
    if (!store) {
      if (deployScoped) {
        if (name) {
          throw createError(
            DRIVER_NAME,
            "deploy-scoped stores cannot have a name"
          );
        }
        store = getDeployStore({ fetch, ...options });
      } else {
        if (!name) {
          throw createRequiredError(DRIVER_NAME, "name");
        }
        store = getStore({ name: encodeURIComponent(name), fetch, ...opts });
      }
    }
    return store;
  };
  return {
    name: DRIVER_NAME,
    options,
    getInstance: getClient,
    async hasItem(key) {
      return getClient().getMetadata(key).then(Boolean);
    },
    getItem: (key, tops) => {
      return getClient().get(key, tops);
    },
    getMeta(key) {
      return getClient().getMetadata(key);
    },
    getItemRaw(key, topts) {
      return getClient().get(key, { type: topts?.type ?? "arrayBuffer" });
    },
    async setItem(key, value, topts) {
      await getClient().set(key, value, topts);
    },
    async setItemRaw(key, value, topts) {
      await getClient().set(key, value, topts);
    },
    removeItem(key) {
      return getClient().delete(key);
    },
    async getKeys(base, tops) {
      return (await getClient().list({ ...tops, prefix: base })).blobs.map(
        (item) => item.key
      );
    },
    async clear(base) {
      const client = getClient();
      return Promise.allSettled(
        (await client.list({ prefix: base })).blobs.map(
          (item) => client.delete(item.key)
        )
      ).then(() => {
      });
    }
  };
});

const netlifyBlobs$1 = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
  __proto__: null,
  default: netlifyBlobs
}, Symbol.toStringTag, { value: 'Module' }));

export { br as b, netlifyBlobs$1 as n, qn as q };
