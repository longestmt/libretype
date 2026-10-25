#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Inspect or register the optional Docs native host; never installs a Chrome extension."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re

import app_bundle

HOST = 'org.libretype.docs'

def plan(app):
    app=app.absolute()
    app_bundle.verify(app)
    helper=app/'Contents/MacOS'/app_bundle.DOCS_EXECUTABLE
    companion=app/'Contents/Resources/GoogleDocsCompanion'
    if not helper.is_file() or not os.access(helper,os.X_OK):
        raise ValueError('This build does not contain the Google Docs companion host')
    manifest=json.loads((companion/'manifest.json').read_text())
    key=base64.b64decode(manifest['key'],validate=True)
    identity=''.join(chr(97+int(c,16)) for c in hashlib.sha256(key).hexdigest()[:32])
    if identity!=(companion/'extension-id.txt').read_text().strip() or not re.fullmatch('[a-p]{32}',identity):
        raise ValueError('The companion identity does not match its public key')
    config={'name':HOST,'description':'Optional LibreType Google Docs geometry bridge','path':str(helper),'type':'stdio','allowed_origins':[f'chrome-extension://{identity}/']}
    return config,companion

def register(destination,config):
    if destination.is_symlink():
        raise ValueError('Refusing a symlink registration')
    if destination.exists():
        if json.loads(destination.read_text())==config:
            return 'already registered'
        raise ValueError('A different registration exists; review it before replacing it')
    destination.parent.mkdir(parents=True,exist_ok=True)
    if destination.parent.is_symlink():
        raise ValueError('Refusing a symlink registration directory')
    fd=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'w') as stream:
        json.dump(config,stream,indent=2);stream.write('\n')
    return 'registered'

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app',type=Path,default=Path.home()/'Applications/LibreType.app')
    parser.add_argument('--install',action='store_true',help='Register the local host after reviewing the plan')
    args=parser.parse_args()
    config,companion=plan(args.app)
    destination=Path.home()/'Library/Application Support/Google/Chrome/NativeMessagingHosts'/f'{HOST}.json'
    result=register(destination,config) if args.install else 'inspection only; no changes'
    print(json.dumps({'result':result,'registration':str(destination),'native_host':config,'chrome_load_unpacked_directory':str(companion)},indent=2))

if __name__=='__main__':
    main()
