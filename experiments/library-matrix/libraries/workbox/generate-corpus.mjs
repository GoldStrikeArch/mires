import {writeFileSync} from 'node:fs'
const first=[{path:'docs/index.html',revision:'1'},{path:'guide.html',revision:'old'},{path:'a?lang=en',revision:'x / 2'}]
const updated=[{path:'docs/index.html',revision:'1'},{path:'guide.html',revision:'new'}]
const install=(manifest,source='alpha')=>({phase:'install',manifest,source})
const activate={phase:'activate'}
const fetch=(path,source='alpha',method='GET')=>({phase:'fetch',path,source,method})
const cases=[]
const add=(id,...steps)=>cases.push({id,input:{steps}})
add('empty',install([]),activate,fetch('unmatched'))
add('duplicate',install([first[0],first[0]]),install([first[0]]),activate)
add('conflict',install([first[0],{...first[0],revision:'2'}]))
add('upgrade',install(first),activate,install(updated,'beta'),activate,fetch('guide.html','offline'))
add('revisionless',install([{path:'bundle.218.js',revision:null}]),activate,fetch('bundle.218.js','offline'))
add('url-variations',install(first),activate,...['docs/','docs/index.html?utm_source=x','guide','guide?fbclid=x','guide?x=y','a?lang=en','a?lang=de'].map(p=>fetch(p,'offline')))
add('method',install(first),fetch('guide.html','offline','POST'),fetch('network/request','alpha','POST'))
add('request-method-casing',install(first),activate,fetch('guide.html','offline','get'),fetch('guide.html','offline','gEt'),fetch('guide.html','offline',''),fetch('guide.html','offline','pOsT'),fetch('network/item','beta','get'))
add('cache-first',fetch('first/item','offline'),fetch('first/item','alpha'),fetch('first/item','offline'),fetch('first/item','beta'))
add('cache-only',fetch('only/item'),fetch('unmatched'))
add('network-only',fetch('network/item','alpha'),fetch('network/item','beta'),fetch('network/item','offline'))
add('removed',install(first),activate,install([]),activate,fetch('docs/','offline'))
add('route-precedence',install([{path:'first/item',revision:'1'}]),activate,fetch('first/item','beta'))
add('bench-cache',install(first),activate,...Array.from({length:24},(_,i)=>fetch(i%3===0?'guide':'first/item-'+i%4, i%2?'alpha':'beta')))
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'workbox',cases})+'\n')
