// Reorder whole sibling subtrees, never individual descendants across parents.
// Bounded deterministic search keeps dense reports usable; the small cache is
// independent of width, selection, direction filters and relation routing.
function packageTreeConnectionsOrder(children, sources, parents, dependencies, root, compareId) {
  if(!dependencies.length||![...children.values()].some(siblings=>siblings.length>1))return;
  const ids=[...sources.keys()].sort(compareId),index=new Map(ids.map((id,i)=>[id,i]));
  const key=JSON.stringify([ids.map(id=>[id,sources.get(id).name,sources.get(id).kind,parents.has(id),parents.get(id)]),dependencies.map(e=>[e.from,e.to])]);
  const cache=packageTreeConnectionsOrder.cache??=new Map();
  const cached=cache.get(key);
  if(cached){const rank=new Map(cached.map((id,i)=>[id,i]));for(const siblings of children.values())siblings.sort((a,b)=>rank.get(a)-rank.get(b));return;}
  function preorder(){
    const order=[],seen=new Set(),pending=[...(children.get(root)||[])].reverse();
    while(pending.length){const id=pending.pop();if(seen.has(id))continue;seen.add(id);order.push(id);for(const child of [...children.get(id)||[]].reverse())pending.push(child);}
    for(const id of ids)if(!seen.has(id))order.push(id);
    return order;
  }
  // Evaluate all edges for ordinary scopes, an evenly spaced sample for large
  // ones. The budget bounds work, not the number of edges eventually drawn.
  const sample=dependencies.length<=256?dependencies:Array.from({length:256},(_,i)=>dependencies[Math.floor(i*dependencies.length/256)]);
  const endpoints=sample.map(e=>[index.get(e.from),index.get(e.to)]),pairs=[];
  for(let i=0;i<endpoints.length;i++)for(let j=i+1;j<endpoints.length;j++){
    const [a,b]=endpoints[i],[c,d]=endpoints[j];if(a!==c&&a!==d&&b!==c&&b!==d)pairs.push(a,b,c,d);
  }
  const positions=new Int32Array(ids.length),scale=ids.length*endpoints.length+1;
  function score(){
    preorder().forEach((id,i)=>positions[index.get(id)]=i);
    let crossings=0,length=0;
    for(let i=0;i<pairs.length;i+=4){
      let lo=positions[pairs[i]],hi=positions[pairs[i+1]],otherLo=positions[pairs[i+2]],otherHi=positions[pairs[i+3]];
      if(lo>hi){const swap=lo;lo=hi;hi=swap;}if(otherLo>otherHi){const swap=otherLo;otherLo=otherHi;otherHi=swap;}
      if(lo<otherLo&&otherLo<hi&&hi<otherHi||otherLo<lo&&lo<otherHi&&otherHi<hi)crossings++;
    }
    for(const [a,b] of endpoints){const span=positions[a]-positions[b];length+=span<0?-span:span;}
    return crossings*scale+length;
  }
  let best=score();const initial=new Map([...children].map(([parent,siblings])=>[parent,[...siblings]]));
  for(const siblings of children.values())siblings.sort(compareId);
  const alphabetical=score();
  if(alphabetical<best)best=alphabetical;else for(const [parent,siblings] of initial)children.set(parent,siblings);
  const groups=[...children.values()].filter(s=>s.length>1).sort((a,b)=>compareId(a[0],b[0]));
  let budget=Math.min(512,Math.max(32,Math.floor(250000/ids.length)));
  for(let pass=0;pass<4&&budget>0;pass++){
    let improved=false;
    for(const siblings of groups){
      for(let gap=1;gap<Math.min(siblings.length,9)&&budget>0;gap++)for(let i=0;i+gap<siblings.length&&budget>0;i++){
        const j=i+gap;[siblings[i],siblings[j]]=[siblings[j],siblings[i]];const candidate=score();budget--;
        if(candidate<best){best=candidate;improved=true;}else [siblings[i],siblings[j]]=[siblings[j],siblings[i]];
      }
      if(!budget)break;
    }
    if(!improved)break;
  }
  if(cache.size>=8)cache.delete(cache.keys().next().value);
  cache.set(key,preorder());
}

// Preserve the lowest-free-lane rule without scanning every occupied lane for
// every edge (Kafka has thousands of overlapping intervals).
function packageTreeAssignLanes(intervals) {
  const active=[],free=[];let count=0;
  const before=(a,b)=>a[0]<b[0]||a[0]===b[0]&&a[1]<b[1];
  function push(heap,value){let at=heap.length;heap.push(value);while(at){const parent=(at-1)>>1;if(!before(value,heap[parent]))break;heap[at]=heap[parent];at=parent;}heap[at]=value;}
  function pop(heap){const first=heap[0],last=heap.pop();if(heap.length){let at=0;while(at*2+1<heap.length){let child=at*2+1;if(child+1<heap.length&&before(heap[child+1],heap[child]))child++;if(!before(heap[child],last))break;heap[at]=heap[child];at=child;}heap[at]=last;}return first;}
  for(const link of intervals){
    while(active.length&&active[0][0]+2 < link.start){const [,lane]=pop(active);push(free,[lane,lane]);}
    link.lane=free.length?pop(free)[0]:count++;push(active,[link.end,link.lane]);
  }
  return count;
}

// Compact exploration tree. Ordering is computed before display filters or
// side mode; Services keeps its existing curved layout unless tracks is set.
function packageTreeLayout(layout, links, viewport={}) {
  if(!layout.nodes.length)return {nodes:[],hierarchy:[],routes:[],width:0,height:0,relationX:0};
  const root=Symbol('forest'),sources=new Map(layout.nodes.map(n=>[n.src,n])),parents=new Map(),children=new Map();
  for(const edge of layout.edges)if(sources.has(edge.from)&&sources.has(edge.to)&&edge.from!==edge.to&&!parents.has(edge.to))parents.set(edge.to,edge.from);
  const compareId=(a,b)=>{
    const x=sources.get(a),y=sources.get(b),kind=(x.kind==='module'?0:1)-(y.kind==='module'?0:1);
    if(kind)return kind;
    return String(x.name)<String(y.name)?-1:String(x.name)>String(y.name)?1:String(a)<String(b)?-1:String(a)>String(b)?1:0;
  };
  for(const node of layout.nodes){const parent=parents.has(node.src)?parents.get(node.src):root;if(!children.has(parent))children.set(parent,[]);children.get(parent).push(node.src);}
  for(const siblings of children.values())siblings.sort(compareId);
  const compareLink=(a,b)=>String(a.key)<String(b.key)?-1:String(a.key)>String(b.key)?1:0;
  const dependencies=links.filter(l=>sources.has(l.from)&&sources.has(l.to)).map(l=>({...l})).sort(compareLink);
  const ancestry=new Map();
  function path(id){
    if(ancestry.has(id))return ancestry.get(id);
    const result=[],seen=new Set();let current=id;
    while(sources.has(current)&&!seen.has(current)){result.push(current);seen.add(current);if(!parents.has(current))break;current=parents.get(current);}
    result.reverse();ancestry.set(id,result);return result;
  }
  // Each dependency orders the two branches below its lowest common ancestor.
  // Thus opening a package does not flatten or scatter its children.
  const groupEdges=new Map();
  for(const edge of dependencies){
    const from=path(edge.from),to=path(edge.to);let i=0;
    while(i<from.length&&i<to.length&&from[i]===to[i])i++;
    let parent,a,b;
    if(edge.from===edge.to){a=b=edge.from;parent=parents.has(a)?parents.get(a):root;}
    else if(i===from.length||i===to.length)continue;
    else {parent=i?from[i-1]:root;a=from[i];b=to[i];}
    if(!groupEdges.has(parent))groupEdges.set(parent,[]);groupEdges.get(parent).push([a,b]);
  }
  const metadata=new Map();
  for(const [parent,siblings] of children){
    const adjacent=new Map(siblings.map(id=>[id,new Set()])),reverse=new Map(siblings.map(id=>[id,new Set()]));
    for(const [a,b] of groupEdges.get(parent)||[])if(adjacent.has(a)&&adjacent.has(b)){adjacent.get(a).add(b);reverse.get(b).add(a);}
    // Iterative Kosaraju: large dependency chains must not consume the JS stack.
    const visited=new Set(),finish=[];
    for(const start of siblings){
      if(visited.has(start))continue;
      visited.add(start);const stack=[{id:start,next:0,neighbors:[...adjacent.get(start)].sort(compareId)}];
      while(stack.length){const frame=stack[stack.length-1];if(frame.next===frame.neighbors.length){finish.push(frame.id);stack.pop();continue;}const next=frame.neighbors[frame.next++];if(!visited.has(next)){visited.add(next);stack.push({id:next,next:0,neighbors:[...adjacent.get(next)].sort(compareId)});}}
    }
    const assigned=new Map(),groups=[];
    for(const start of finish.reverse()){
      if(assigned.has(start))continue;
      const group=[],gid=groups.length,pending=[start];assigned.set(start,gid);
      while(pending.length){const id=pending.pop();group.push(id);for(const next of reverse.get(id))if(!assigned.has(next)){assigned.set(next,gid);pending.push(next);}}
      group.sort(compareId);groups.push(group);
    }
    const outgoing=groups.map(()=>new Set()),incoming=groups.map(()=>0);
    for(const [a,targets] of adjacent)for(const b of targets){const ga=assigned.get(a),gb=assigned.get(b);if(ga!==gb&&!outgoing[ga].has(gb)){outgoing[ga].add(gb);incoming[gb]++;}}
    const compareGroup=(a,b)=>compareId(groups[a][0],groups[b][0]),ready=groups.map((_,i)=>i).filter(i=>incoming[i]===0),ordered=[];
    while(ready.length){ready.sort(compareGroup);const gid=ready.shift();ordered.push(gid);for(const next of outgoing[gid])if(--incoming[next]===0)ready.push(next);}
    const levels=groups.map(()=>0);
    for(const gid of [...ordered].reverse())for(const next of outgoing[gid])levels[gid]=Math.max(levels[gid],levels[next]+1);
    // Keep construction levels contiguous, including disconnected siblings.
    ordered.sort((a,b)=>levels[b]-levels[a]||compareGroup(a,b));
    const order=[];
    for(const gid of ordered){const cycle=groups[gid].length>1||groups[gid].some(id=>adjacent.get(id).has(id));for(const id of groups[gid]){metadata.set(id,{level:levels[gid],siblingCycle:cycle,siblingGroup:groups[gid][0]});order.push(id);}}
    children.set(parent,order);
  }
  if(viewport?.order==='alphabetical')for(const siblings of children.values())siblings.sort(compareId);
  else if(viewport?.order==='connections')packageTreeConnectionsOrder(children,sources,parents,dependencies,root,compareId);
  const ordered=[],seen=new Set(),pending=[...(children.get(root)||[])].reverse().map(id=>({id,depth:0}));
  while(pending.length){const item=pending.pop();if(seen.has(item.id))continue;seen.add(item.id);ordered.push(item);for(const id of [...children.get(item.id)||[]].reverse())pending.push({id,depth:item.depth+1});}
  // Defensive malformed containment fallback, preserving finite output.
  for(const source of layout.nodes)if(!seen.has(source.src))ordered.push({id:source.src,depth:0});
  const maxDepth=Math.max(...ordered.map(n=>n.depth)),minLabelWidth=Math.max(400,300+maxDepth*18),left=24,baseGutter=194;
  // Keep readable row sizes on narrow screens. Wider panes give labels more
  // room and spread relations, without changing font sizes or row order.
  const minimumWidth=minLabelWidth+left*3+baseGutter*2;
  const extra=Number.isFinite(viewport?.width)?Math.max(0,viewport.width-minimumWidth):0;
  const labelExtra=Math.min(Math.max(0,700-minLabelWidth),extra*.4),w=minLabelWidth+labelExtra;
  const gutter=baseGutter+(extra-labelExtra)/2,tracks=viewport?.tracks===true,compact=tracks&&viewport?.directional!==true;
  // Tree labels sit between two gutters. Keep the same row positions when
  // switching routing or filtering; legacy Services retains its single axis.
  const labelLeft=tracks?gutter+36:left,relationX=tracks?labelLeft+w+24:left+w+24+gutter;
  const incomingX=tracks?labelLeft-24:relationX;
  const nodes=ordered.map(({id,depth},index)=>({...sources.get(id),...metadata.get(id),depth,indent:depth*18,w,h:28,x:labelLeft+w/2,y:24+14+index*30,relationX,incomingX}));
  const byId=new Map(nodes.map(n=>[n.src,n]));
  const intervals=dependencies.map(link=>{const from=byId.get(link.from),to=byId.get(link.to);return {...link,sourceY:from.y+3,targetY:to.y-3,start:Math.min(from.y+3,to.y-3),end:Math.max(from.y+3,to.y-3)};});
  intervals.sort((a,b)=>a.start-b.start||a.end-b.end||compareLink(a,b));
  const laneCount=packageTreeAssignLanes(intervals);
  const routes=intervals.map(link=>{
    const from=byId.get(link.from),to=byId.get(link.to),side=link.side==='left'||link.side==='right'?link.side:compact?'right':from.y<to.y?'left':'right',sign=side==='left'?-1:1;
    const spread=laneCount>1?link.lane/(laneCount-1):0;
    const reach=(Math.min(166,48+Math.sqrt(Math.abs(link.targetY-link.sourceY))*5)+spread*28)*(gutter/baseGutter);
    const axisX=side==='left'?incomingX:relationX;
    const source={x:axisX+sign*8,y:link.sourceY},lead={x:axisX+sign*20,y:link.sourceY},control1={x:axisX+sign*reach,y:link.sourceY},control2={x:axisX+sign*reach,y:link.targetY},tail={x:axisX+sign*20,y:link.targetY},target={x:axisX+sign*8,y:link.targetY};
    const path=treeDependencyPath([source,lead,control1,control2,tail,target]);
    return {key:link.key,from:link.from,to:link.to,side,lane:link.lane,points:[source,lead,control1,control2,tail,target],path};
  }).sort(compareLink);
  const hierarchy=layout.edges.filter(e=>byId.has(e.from)&&byId.has(e.to)&&parents.get(e.to)===e.from).map(edge=>{
    const from=byId.get(edge.from),to=byId.get(edge.to),rail=labelLeft+from.indent+9;
    return {from:edge.from,to:edge.to,points:[{x:rail,y:from.y+8},{x:rail,y:to.y},{x:labelLeft+to.indent+7,y:to.y}]};
  });
  return {nodes,hierarchy,routes,width:relationX+gutter+(tracks?12:24),height:nodes.at(-1).y+14+24,relationX,incomingX};
}

// Round non-collinear bends. Half-segment limits leave both endpoint
// tangents intact and prevent neighboring corners from overshooting each other.
// The curve stays in the corner's convex hull, inside the existing free gutter.
function roundedTreePath(points, radius=10) {
  const path=points.filter((p,i)=>!i||p.x!==points[i-1].x||p.y!==points[i-1].y);
  if(!path.length)return '';
  const limit=Number.isFinite(radius)?Math.max(0,radius):0;
  let result='M '+path[0].x+' '+path[0].y;
  for(let i=1;i<path.length-1;i++){
    const before=path[i-1],corner=path[i],after=path[i+1];
    const turn=(corner.x-before.x)*(after.y-corner.y)-(corner.y-before.y)*(after.x-corner.x);
    if(!turn||!limit){result+=' L '+corner.x+' '+corner.y;continue;}
    const incoming=Math.hypot(corner.x-before.x,corner.y-before.y),outgoing=Math.hypot(after.x-corner.x,after.y-corner.y);
    const r=Math.min(limit,incoming/2,outgoing/2);
    const entry={x:corner.x-(corner.x-before.x)*r/incoming,y:corner.y-(corner.y-before.y)*r/incoming};
    const exit={x:corner.x+(after.x-corner.x)*r/outgoing,y:corner.y+(after.y-corner.y)*r/outgoing};
    result+=' L '+entry.x+' '+entry.y+' Q '+corner.x+' '+corner.y+' '+exit.x+' '+exit.y;
  }
  if(path.length>1){const last=path[path.length-1];result+=' L '+last.x+' '+last.y;}
  return result;
}

// Original smooth relationship arc: horizontal leads and a continuous cubic bow.
function treeDependencyPath([source,lead,c1,c2,tail,target]){
  return `M ${source.x} ${source.y} L ${lead.x} ${lead.y} C ${c1.x} ${c1.y} ${c2.x} ${c2.y} ${tail.x} ${tail.y} L ${target.x} ${target.y}`;
}
function treeCurvePoint(points,t){
  const [,a,b,c,d]=points,u=1-t;
  return {x:u*u*u*a.x+3*u*u*t*b.x+3*u*t*t*c.x+t*t*t*d.x,y:u*u*u*a.y+3*u*u*t*b.y+3*u*t*t*c.y+t*t*t*d.y};
}
