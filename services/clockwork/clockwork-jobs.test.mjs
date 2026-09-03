import assert from "node:assert";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { spawnSync } from "node:child_process";
import { inspectManifest } from "./clockwork-jobs.mjs";

const SCRIPT = new URL("./clockwork-jobs.mjs", import.meta.url).pathname;
function tree(root) { const h=crypto.createHash("sha256"); if(!fs.existsSync(root))return "missing"; const walk=(p)=>{for(const n of fs.readdirSync(p).sort()){const q=path.join(p,n),s=fs.lstatSync(q);h.update(q+":"+s.mode);if(s.isDirectory())walk(q);else h.update(fs.readFileSync(q));}};walk(root);return h.digest("hex"); }
function manifest(name, action, paused=true) { return `name: ${name}\njobs:\n  ${name}:\n    schedule: "0 9 * * *"\n    ${action}\n    paused: ${paused}\n`; }
function setup() {
  const tmp=fs.mkdtempSync(path.join(os.tmpdir(),"clockwork-jobs-")), home=path.join(tmp,"home"), jobs=path.join(home,".agents/clockwork/jobs.d"), state=path.join(home,".local/state/clockwork"); fs.mkdirSync(jobs,{recursive:true});
  const fake=path.join(tmp,"clockwork"); fs.writeFileSync(fake,`#!/usr/bin/env node
const fs=require('fs'),path=require('path'),root=process.env.CLOCKWORK_HOME,file=path.join(root,'fake.json');let s=fs.existsSync(file)?JSON.parse(fs.readFileSync(file)):{agents:[],jobs:[{name:'unmanaged',manifest:'other'}],calls:[]};const a=process.argv.slice(2),save=()=>{fs.mkdirSync(root,{recursive:true});fs.writeFileSync(file,JSON.stringify(s))};
if(a[0]==='agent'&&a[1]==='list'){console.log(JSON.stringify(s.agents));process.exit()};if(a[0]==='list'){console.log(JSON.stringify(s.jobs));process.exit()};if(a[0]==='up'){if(a.includes('--dry-run')){console.log('{}');process.exit()};const f=a[a.indexOf('--file')+1],name=fs.readFileSync(f,'utf8').match(/^name:\\s*(\\S+)/m)[1];s.jobs=s.jobs.filter(j=>j.manifest!==name);s.jobs.push({name,manifest:name});s.calls.push(['up',name]);save();process.exit()};if(a[0]==='down'){const n=a[a.indexOf('--manifest')+1];s.jobs=s.jobs.filter(j=>j.manifest!==n);s.calls.push(['down',n]);save();process.exit()};if(a[0]==='agent'&&a[1]==='add'){const n=a.at(-1),bin=a[a.indexOf('--bin')+1],args=[];for(let i=0;i<a.length;i++){if(a[i]==='--arg')args.push(a[i+1]);if(a[i].startsWith('--arg='))args.push(a[i].slice('--arg='.length));}s.agents=s.agents.filter(x=>x.name!==n);s.agents.push({name:n,bin,args,prompt_stdin:a.includes('--prompt-stdin')});s.calls.push(['agent-add',n]);save();process.exit()};if(a[0]==='agent'&&a[1]==='rm'){s.agents=s.agents.filter(x=>x.name!==a[2]);s.calls.push(['agent-rm',a[2]]);save();process.exit()};process.exit(2);`); fs.chmodSync(fake,0o755);
  const env={...process.env,HOME:home,CLOCKWORK_HOME:state,CLOCKWORK_JOBS_ROOT:jobs,CLOCKWORK_BIN:fake,CLOCKWORK_PI_BIN:path.join(home,'.local/bin/clockwork-pi')};
  const add=(name,text,profile)=>{const d=path.join(jobs,name);fs.mkdirSync(d,{recursive:true});fs.writeFileSync(path.join(d,'clockwork.yaml'),text);if(profile)fs.writeFileSync(path.join(d,'pi-profile.json'),JSON.stringify(profile));};
  const run=(args)=>spawnSync(process.execPath,[SCRIPT,...args],{env,encoding:'utf8'}); return {tmp,home,jobs,state,fake,env,add,run};
}

describe("clockwork job reconciler",()=>{
  it("validates identity and secure action types",()=>{
    assert.strictEqual(inspectManifest(manifest("cmd","run: \"true\""),"cmd").action,"command");
    assert.strictEqual(inspectManifest(manifest("hook","webhook: \"https://example.invalid/hook\""),"hook").action,"webhook");
    assert.throws(()=>inspectManifest(manifest("hook","webhook: \"http://example.test/hook\""),"hook"),/HTTPS/);
    assert.throws(()=>inspectManifest(manifest("wrong","run: \"true\""),"right"),/manifest name/);
  });
  it("creates paused jobs, keeps stable profiles, updates, pauses, resumes, and prunes only owned state",()=>{
    const x=setup(), cwd=path.join(x.tmp,"project");fs.mkdirSync(cwd);const p={version:1,cwd,model:"provider/model",thinking:"high",tools:["read"],approveProjectFiles:false};
    x.add("cmd",manifest("cmd","run: \"true\""));x.add("prompt",manifest("prompt","agent: clockwork-pi-prompt\n    prompt: \"hello\""),p);x.add("hook",manifest("hook","webhook: \"https://example.invalid/hook\""));
    const before=tree(x.home);let r=x.run(["check","--json"]);assert.strictEqual(r.status,0,r.stderr);assert.strictEqual(tree(x.home),before);
    r=x.run(["plan","--json"]);assert.strictEqual(r.status,0,r.stderr);assert.strictEqual(tree(x.home),before);
    r=x.run(["apply","--confirm","all","--json","--no-input"]);assert.strictEqual(r.status,0,r.stderr);let s=JSON.parse(fs.readFileSync(path.join(x.state,"fake.json")));assert.ok(s.jobs.some(j=>j.manifest==='other'));assert.deepStrictEqual(s.agents[0].args,["--job","prompt"]);
    const ownership=fs.readFileSync(path.join(x.state,"integration/ownership.json"));r=x.run(["apply","--confirm","all"]);assert.strictEqual(r.status,0,r.stderr);assert.deepStrictEqual(fs.readFileSync(path.join(x.state,"integration/ownership.json")),ownership);
    fs.writeFileSync(path.join(x.jobs,"cmd/clockwork.yaml"),manifest("cmd","run: \"printf updated\"",false));r=x.run(["apply","cmd","--confirm","cmd"]);assert.strictEqual(r.status,0,r.stderr);
    fs.writeFileSync(path.join(x.jobs,"cmd/clockwork.yaml"),manifest("cmd","run: \"printf updated\"",true));assert.strictEqual(x.run(["apply","cmd","--confirm","cmd"]).status,0);
    fs.rmSync(path.join(x.jobs,"prompt"),{recursive:true});r=x.run(["apply","--confirm","all"]);assert.strictEqual(r.status,0,r.stderr);s=JSON.parse(fs.readFileSync(path.join(x.state,"fake.json")));assert.ok(s.calls.some(c=>c[0]==='down'&&c[1]==='prompt'));assert.ok(s.calls.some(c=>c[0]==='agent-rm'));assert.ok(s.jobs.some(j=>j.manifest==='other'));assert.ok(fs.existsSync(path.join(x.state,"fake.json")));
    fs.rmSync(x.tmp,{recursive:true,force:true});
  });
  it("rejects active-first, confirmation bypass, collisions, and fails closed without ownership",()=>{
    const x=setup();x.add("active",manifest("active","run: \"true\"",false));let r=x.run(["apply","--confirm","all"]);assert.strictEqual(r.status,1);assert.match(r.stderr,/first apply/);
    r=x.run(["apply"]);assert.strictEqual(r.status,2);assert.match(r.stderr,/--confirm all/);
    fs.mkdirSync(x.state,{recursive:true});fs.writeFileSync(path.join(x.state,"fake.json"),JSON.stringify({agents:[],jobs:[{manifest:'active'}],calls:[]}));fs.writeFileSync(path.join(x.jobs,"active/clockwork.yaml"),manifest("active","run: \"true\"",true));r=x.run(["plan"]);assert.strictEqual(r.status,1);assert.match(r.stderr,/unmanaged manifest collision/);
    fs.rmSync(x.tmp,{recursive:true,force:true});
  });
  it("executes when invoked through an installed symlink",()=>{
    const x=setup();x.add("cmd",manifest("cmd","run: \"true\""));const installed=path.join(x.tmp,"clockwork-jobs");fs.symlinkSync(SCRIPT,installed);const r=spawnSync(installed,["check","--json"],{env:x.env,encoding:"utf8"});assert.strictEqual(r.status,0,r.stderr);assert.deepStrictEqual(JSON.parse(r.stdout).jobs,["cmd"]);fs.rmSync(x.tmp,{recursive:true,force:true});
  });
});
