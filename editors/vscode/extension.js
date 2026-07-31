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
