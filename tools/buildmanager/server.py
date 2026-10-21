"""Versioned JSONL service, serial build queue and supervised child lifetimes."""
import ctypes
import json
import os
import signal
import struct
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

import build_rs as build
from .model import VERSION, Project, WorkspaceLock, capabilities, read_json, text, write_json
from .runner import build_arguments, cache_path


class ManagedProcess:
    def __init__(self, command, env):
        self.handle = None
        self.process = subprocess.Popen(command, cwd=build.ROOT, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=os.name != 'nt',
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        try:
            if os.name == 'nt':
                k = ctypes.WinDLL('kernel32', use_last_error=True)
                k.CreateJobObjectW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
                k.CreateJobObjectW.restype = ctypes.c_void_p
                k.SetInformationJobObject.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
                k.AssignProcessToJobObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
                k.TerminateJobObject.argtypes = [ctypes.c_void_p, ctypes.c_uint]
                k.CloseHandle.argtypes = [ctypes.c_void_p]
                self.kernel = k
                self.handle = k.CreateJobObjectW(None, None)
                limits = ctypes.create_string_buffer(144)
                struct.pack_into('<I', limits, 16, 0x2000)
                if not self.handle or not k.SetInformationJobObject(self.handle, 9, limits, 144) or not k.AssignProcessToJobObject(self.handle, int(self.process._handle)):
                    raise OSError(ctypes.get_last_error(), 'Prozessgruppe konnte nicht erstellt werden')
            # Worker cannot spawn descendants before assignment to its Windows job.
            self.process.stdin.write(b'go\n')
            self.process.stdin.flush()
        except BaseException:
            self.process.kill()
            self.process.wait()
            self.close()
            raise

    def terminate(self):
        if os.name == 'nt':
            if self.handle:
                self.kernel.TerminateJobObject(self.handle, 1)
        else:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def close(self):
        self.terminate()
        self.process.wait()
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None
        for stream in (self.process.stdin, self.process.stdout):
            if stream:
                stream.close()


class Server:
    def __init__(self, root, output=None, distro='Ubuntu-24.04'):
        self.project = Project(root)
        self.lock = WorkspaceLock(root)
        self.output = output or sys.stdout
        self.output_lock = threading.Lock()
        self.condition = threading.Condition(threading.RLock())
        self.distro = distro
        self.caps = None
        self.directory = self.project.root / 'build/manager/jobs'
        self.directory.mkdir(parents=True, exist_ok=True)
        self.jobs = {}
        self.queue = []
        self.processes = {}
        self.threads = []
        self.stopping = False
        for path in sorted(self.directory.glob('*.json')):
            try:
                job = read_json(path, {})
                if job.get('status') in ('queued', 'running'):
                    job['status'] = 'interrupted'
                    job['message'] = 'Beim letzten Beenden unterbrochen; erneut ausführen ist möglich.'
                    write_json(path, job)
                self.jobs[job['id']] = job
            except (OSError, ValueError, KeyError):
                continue
        self.scheduler = threading.Thread(target=self.schedule, daemon=True)
        self.scheduler.start()

    def emit(self, **value):
        with self.output_lock:
            self.output.write(json.dumps(dict(version=VERSION, **value), ensure_ascii=False) + '\n')
            self.output.flush()

    def persist(self, job):
        write_json(self.directory / (job['id'] + '.json'), job)

    def summary(self):
        return [self.public(j, compact=True) for j in
                sorted(self.jobs.values(), key=lambda j: j['created'])[-200:]]

    def refresh(self):
        self.caps = capabilities(self.distro)
        value = self.project.catalog(self.caps)
        with self.condition:
            value['jobs'] = self.summary()
        return value

    def add(self, spec, launch=False):
        ident = time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:10]
        spec = dict(spec, id=ident, wsl_distro=self.distro)
        job = dict(id=ident, app=spec['app'], status='queued', phase='Warteschlange', created=time.time(),
                   mode='run' if launch else spec['mode'], request=spec, artifacts=[], tests=[], message='')
        self.jobs[ident] = job
        self.persist(job)
        self.emit(event='job', job=self.public(job))
        if launch:
            thread = threading.Thread(target=self.work, args=(ident,), daemon=True)
            self.threads.append(thread)
            thread.start()
        else:
            self.queue.append(ident)
            self.condition.notify_all()
        return ident

    @staticmethod
    def public(job, compact=False):
        data = {k: v for k, v in job.items() if k != 'request'}
        data['elapsed'] = job.get('duration', time.time() - job.get('started', time.time()))
        if compact:
            data['tests'] = []
        else:
            data['tests'] = [{k: v for k, v in test.items() if k != 'output'} for test in job.get('tests', [])]
        return data

    def dispatch(self, method, p):
        if method == 'catalog':
            self.distro = text(p.get('wsl_distro', self.distro), 'WSL-Distribution', 128)
            return self.refresh()
        if method == 'save_package':
            return dict(app=p['app'], **self.project.save_package(p['app'], p['values'], p['revision']))
        if method == 'save_profile':
            return self.project.save_profile(p['kind'], p['profile'], p['revision'])
        if method in ('preview', 'enqueue'):
            if self.caps is None:
                self.refresh()
            specs = self.project.resolve(p['profile'], self.caps)
            if method == 'preview':
                return dict(profile=self.project.build_profile(p['profile']),
                            jobs=[dict(spec=s, commands=build_arguments(s, cache_path(s))) for s in specs])
            with self.condition:
                return dict(ids=[self.add(s) for s in specs])
        if method == 'job':
            with self.condition:
                job = dict(self.jobs[p['id']])
                job['tests'] = [{k: v for k, v in t.items() if k != 'output'} for t in job.get('tests', [])]
            log = self.directory / (job['id'] + '.log')
            if log.exists():
                with log.open('rb') as stream:
                    stream.seek(max(0, log.stat().st_size - 65536))
                    job['log'] = stream.read().decode('utf-8', 'replace')
            return job
        if method == 'cancel':
            with self.condition:
                job = self.jobs[p['id']]
                if job['status'] in ('queued', 'running'):
                    job['status'] = 'cancelled'
                    job['message'] = 'Vom Benutzer abgebrochen.'
                    if job['id'] in self.queue:
                        self.queue.remove(job['id'])
                    if job['id'] in self.processes:
                        self.processes[job['id']].terminate()
                    self.persist(job)
                    self.emit(event='job', job=self.public(job))
            return dict(status=job['status'])
        if method == 'retry':
            with self.condition:
                spec = self.jobs[p['id']]['request']
            if spec.get('launch'):
                raise ValueError('Bitte das gewünschte Ergebnis erneut starten.')
            specs = self.project.resolve(spec, self.caps or capabilities(self.distro))
            with self.condition:
                return dict(ids=[self.add(s) for s in specs])
        if method == 'run':
            with self.condition:
                job = self.jobs[p['id']]
                if job['status'] != 'succeeded':
                    raise ValueError('Nur erfolgreiche Ergebnisse können gestartet werden.')
                index = p['artifact']
                if type(index) is not int or not 0 <= index < len(job['artifacts']):
                    raise ValueError('Ungültiges Ergebnis.')
                artifact = job['artifacts'][index]
                if artifact['kind'] != 'app' or not Path(artifact['path']).is_file():
                    raise ValueError('Dieses Ergebnis ist keine ausführbare native App.')
                if not (self.caps or capabilities(self.distro))['targets'][artifact['target']]['run']:
                    raise ValueError('Dieses Ergebnis kann auf diesem Rechner nicht ausgeführt werden.')
                spec = dict(job['request'], launch=dict(artifact=artifact, argv=self.project.arguments(p.get('argv', []))))
                return dict(id=self.add(spec, launch=True))
        raise ValueError('Unbekannte Operation: ' + str(method))

    def schedule(self):
        while True:
            with self.condition:
                self.condition.wait_for(lambda: self.stopping or self.queue)
                if self.stopping:
                    return
                ident = self.queue.pop(0)
            self.work(ident)

    def work(self, ident):
        managed = None
        with self.condition:
            job = self.jobs[ident]
            if job['status'] != 'queued' or self.stopping:
                return
            job.update(status='running', phase='Starten', started=time.time())
            self.persist(job)
            self.emit(event='job', job=self.public(job))
        try:
            request = self.directory / (ident + '.request')
            write_json(request, job['request'])
            env = dict(os.environ, PYTHONUNBUFFERED='1', PYTHONIOENCODING='utf-8', jfgui_WSL_DISTRO=job['request']['wsl_distro'])
            with self.condition:
                if job['status'] == 'cancelled' or self.stopping:
                    return
                managed = ManagedProcess([sys.executable, '-u', str(build.ROOT / 'tools/build_manager.py'), '--job', str(request)], env)
                self.processes[ident] = managed
            with (self.directory / (ident + '.log')).open('a', encoding='utf-8') as log:
                for raw in managed.process.stdout:
                    line = raw.decode('utf-8', 'replace')
                    try:
                        event = json.loads(line)
                    except ValueError:
                        event = dict(event='log', text=line)
                    with self.condition:
                        kind = event.get('event')
                        if kind == 'phase':
                            job['phase'] = event['phase']
                        elif kind == 'result':
                            job['artifacts'] = event.get('artifacts', [])
                            job['tests'] = event.get('tests', [])
                        elif kind == 'test':
                            job['tests'].append({k: v for k, v in event.items() if k != 'event'})
                        elif kind == 'failure':
                            job['message'] = event['message']
                        if kind == 'log':
                            log.write(event['text'])
                            log.flush()
                        else:
                            log.write(json.dumps(event, ensure_ascii=False) + '\n')
                            log.flush()
                            self.persist(job)
                        self.emit(job_id=ident, **event)
            code = managed.process.wait()
            with self.condition:
                if job['status'] != 'cancelled':
                    job['status'] = 'succeeded' if code == 0 else 'failed'
                if job['status'] in ('succeeded', 'cancelled'):
                    job['phase'] = 'Abgeschlossen' if job['status'] == 'succeeded' else 'Abgebrochen'
                if code and not job['message']:
                    job['message'] = f'Prozess beendet mit Exitcode {code}.'
        except Exception as e:
            with self.condition:
                if job['status'] != 'cancelled':
                    job.update(status='failed', message=str(e))
        finally:
            if managed:
                managed.close()
            with self.condition:
                self.processes.pop(ident, None)
                job['finished'] = time.time()
                job['duration'] = job['finished'] - job.get('started', job['created'])
                self.persist(job)
                self.emit(event='job', job=self.public(job))

    def close(self):
        with self.condition:
            self.stopping = True
            for job in self.jobs.values():
                if job['status'] in ('queued', 'running'):
                    job.update(status='cancelled', message='Build Manager geschlossen.')
                    self.persist(job)
            for managed in self.processes.values():
                managed.terminate()
            self.condition.notify_all()
        self.scheduler.join()
        for thread in self.threads:
            thread.join()
        self.lock.close()


def serve(distro):
    try:
        server = Server(build.ROOT, distro=distro)
    except (OSError, ValueError) as error:
        print(json.dumps(dict(version=VERSION, id=None, method='catalog', error=str(error)), ensure_ascii=False), flush=True)
        return 1
    try:
        while True:
            raw = sys.stdin.buffer.readline(1048577)
            if not raw:
                break
            ident = None
            method = ''
            try:
                if len(raw) > 1048576:
                    while raw and not raw.endswith(b'\n'):
                        raw = sys.stdin.buffer.readline(1048577)
                    raise ValueError('Anfrage überschreitet 1 MiB.')
                value = json.loads(raw)
                if not isinstance(value, dict) or value.get('version') != VERSION:
                    raise ValueError('Unbekannte Protokollversion.')
                ident = value.get('id')
                method = value.get('method')
                params = value.get('params', {})
                if not isinstance(method, str) or not isinstance(params, dict):
                    raise ValueError('Operation und Parameter haben ein ungültiges Format.')
                result = server.dispatch(method, params)
                server.emit(id=ident, method=method, result=result)
            except (ValueError, KeyError, TypeError, OSError) as e:
                server.emit(id=ident, method=method, error=str(e))
    finally:
        server.close()
    return 0
