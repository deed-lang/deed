'use strict';

const assert = require('node:assert/strict');
const path = require('node:path');
const test = require('node:test');
const Module = require('node:module');

function loadWithMocks(configValues) {
  const originalLoad = Module._load;
  const createdClients = [];

  class MockLanguageClient {
    constructor(id, name, serverOptions, clientOptions) {
      this.id = id;
      this.name = name;
      this.serverOptions = serverOptions;
      this.clientOptions = clientOptions;
      this.started = false;
      this.stopped = false;
      createdClients.push(this);
    }

    start() {
      this.started = true;
      return { dispose() {} };
    }

    stop() {
      this.stopped = true;
      return Promise.resolve();
    }
  }

  const registeredAdapters = [];

  class MockDebugAdapterExecutable {
    constructor(command, args) {
      this.command = command;
      this.args = args;
    }
  }

  const mockedVscode = {
    workspace: {
      getConfiguration() {
        return {
          get(key, fallback) {
            return Object.prototype.hasOwnProperty.call(configValues, key)
              ? configValues[key]
              : fallback;
          },
        };
      },
    },
    debug: {
      registerDebugAdapterDescriptorFactory(type, factory) {
        registeredAdapters.push({ type, factory });
        return { dispose() {} };
      },
    },
    DebugAdapterExecutable: MockDebugAdapterExecutable,
  };

  Module._load = function mockedLoad(request, parent, isMain) {
    if (request === 'vscode') {
      return mockedVscode;
    }
    if (request === 'vscode-languageclient/node') {
      return {
        LanguageClient: MockLanguageClient,
        TransportKind: { stdio: 'stdio' },
      };
    }
    return originalLoad.call(this, request, parent, isMain);
  };

  const extensionPath = path.join(__dirname, '..', 'extension.js');
  delete require.cache[require.resolve(extensionPath)];

  const extension = require(extensionPath);

  return {
    extension,
    createdClients,
    registeredAdapters,
    restore() {
      Module._load = originalLoad;
      delete require.cache[require.resolve(extensionPath)];
    },
  };
}

test('activate starts Deed language client with default command and stdio', async () => {
  const loaded = loadWithMocks({ 'server.args': [] });
  try {
    const context = { subscriptions: [] };
    loaded.extension.activate(context);

    assert.equal(loaded.createdClients.length, 1);
    const client = loaded.createdClients[0];
    assert.equal(client.id, 'deed-lsp');
    assert.equal(client.started, true);
    assert.equal(client.serverOptions.command, 'deed');
    assert.deepEqual(client.serverOptions.args, ['lsp']);
    assert.equal(client.serverOptions.transport, 'stdio');
    assert.deepEqual(client.clientOptions.documentSelector, [
      { language: 'deed', scheme: 'file' },
      { language: 'deed', scheme: 'untitled' },
    ]);
    assert.equal(context.subscriptions.length, 2);

    await loaded.extension.deactivate();
    assert.equal(client.stopped, true);
  } finally {
    loaded.restore();
  }
});

test('activate applies configured server path and extra args after lsp', () => {
  const loaded = loadWithMocks({
    'server.path': '/tmp/deed',
    'server.args': ['--trace', 'verbose'],
  });

  try {
    loaded.extension.activate({ subscriptions: [] });

    assert.equal(loaded.createdClients.length, 1);
    const client = loaded.createdClients[0];
    assert.equal(client.serverOptions.command, '/tmp/deed');
    assert.deepEqual(client.serverOptions.args, ['lsp', '--trace', 'verbose']);
  } finally {
    loaded.restore();
  }
});

test('activate registers a debug adapter for deed', () => {
  const loaded = loadWithMocks({});

  try {
    loaded.extension.activate({ subscriptions: [] });

    assert.equal(loaded.registeredAdapters.length, 1);
    assert.equal(loaded.registeredAdapters[0].type, 'deed');
  } finally {
    loaded.restore();
  }
});

// The adapter is the same binary as the server, started with `debug`. Two
// settings pointing at one executable would let an editor talk to two versions
// of the compiler at once, which is a class of bug nobody would think to look
// for.
test('the debug adapter is the configured executable, started with debug', () => {
  const loaded = loadWithMocks({ 'server.path': '/tmp/deed' });

  try {
    loaded.extension.activate({ subscriptions: [] });

    const { factory } = loaded.registeredAdapters[0];
    const descriptor = factory.createDebugAdapterDescriptor();
    assert.equal(descriptor.command, '/tmp/deed');
    assert.deepEqual(descriptor.args, ['debug']);
  } finally {
    loaded.restore();
  }
});

// `server.args` is for the language server. Passing them to the adapter as
// well would send `--trace verbose` to a command that does not take it, and
// the failure would look like a debugger that will not start.
test('language server arguments do not reach the debug adapter', () => {
  const loaded = loadWithMocks({ 'server.args': ['--trace', 'verbose'] });

  try {
    loaded.extension.activate({ subscriptions: [] });

    const { factory } = loaded.registeredAdapters[0];
    assert.deepEqual(factory.createDebugAdapterDescriptor().args, ['debug']);
  } finally {
    loaded.restore();
  }
});
