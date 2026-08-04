'use strict';

const vscode = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

let client;

function serverCommand() {
  const settings = vscode.workspace.getConfiguration('deed');
  const command = settings.get('server.path', 'deed');
  const extra = settings.get('server.args', []);
  return {
    command,
    args: ['lsp', ...extra],
  };
}

/// The adapter is the same binary as the server, started with `debug` instead
/// of `lsp`. One setting rather than two, because two paths to the same
/// executable is two chances for an editor to be talking to two versions of
/// the compiler.
function debugAdapterFactory() {
  return {
    createDebugAdapterDescriptor() {
      const settings = vscode.workspace.getConfiguration('deed');
      const command = settings.get('server.path', 'deed');
      return new vscode.DebugAdapterExecutable(command, ['debug']);
    },
  };
}

function activate(context) {
  const launch = serverCommand();
  const serverOptions = {
    command: launch.command,
    args: launch.args,
    transport: TransportKind.stdio,
  };
  const clientOptions = {
    documentSelector: [
      { language: 'deed', scheme: 'file' },
      { language: 'deed', scheme: 'untitled' },
    ],
  };

  client = new LanguageClient('deed-lsp', 'Deed Language Server', serverOptions, clientOptions);
  context.subscriptions.push(client.start());
  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory('deed', debugAdapterFactory()),
  );
}

function deactivate() {
  if (!client) {
    return undefined;
  }

  const running = client;
  client = undefined;
  return running.stop();
}

module.exports = {
  activate,
  deactivate,
};
