'use strict';
// Camera uses pixels relative to the fitted view's center. Wheel zoom preserves
// the world point below the cursor, including at both zoom limits.
function worldZoom(camera,factor,point){
  const zoom=Math.max(.4,Math.min(8,camera.zoom*factor)),ratio=zoom/camera.zoom;
  return {...camera,x:point.x-(point.x-camera.x)*ratio,y:point.y-(point.y-camera.y)*ratio,zoom};
}
// Packets show a call or event direction, so configuration references and
// startup order never carry them. They keep a constant screen speed and fade at
// both ends of the curve so they appear to leave and enter the plates.
function worldFlowCarrier(kind){return ['http_call','grpc_call','event','configured_http'].includes(kind);}
function worldFlowPackets(p,control,q,flow,offset=0){
  const length=Math.max(1,Math.hypot(q.x-p.x,q.y-p.y)),count=Math.max(1,Math.min(4,Math.round(length/90)));
  return Array.from({length:count},(_,i)=>{
    const t=(flow*60/length+offset+i/count)%1,s=1-t;
    return {x:s*s*p.x+2*s*t*control.x+t*t*q.x,y:s*s*p.y+2*s*t*control.y+t*t*q.y,alpha:Math.sin(Math.PI*t)};
  });
}
function worldRotate(camera,delta){const turn=Math.PI*2;return {...camera,rotation:(((camera.rotation||0)+delta)%turn+turn)%turn};}
// A two-contact gesture combines scale, bearing and centroid movement. Undo
// the isometric projection before rotating the anchor so the ground point
// between the fingers stays put, including at either zoom limit.
function worldGesture(camera,previous,current,groundLift=0){
  if(previous.distance<8||current.distance<8)return {...camera};
  const delta=Math.atan2(Math.sin(current.angle-previous.angle),Math.cos(current.angle-previous.angle));
  const next=worldRotate(worldZoom(camera,current.distance/previous.distance,previous.center),delta),ratio=next.zoom/camera.zoom;
  const px=(previous.center.x-camera.x)/.86,py=(previous.center.y-camera.y-groundLift*camera.zoom)/.44;
  const anchor=worldProject((px+py)/2,(py-px)/2,0,delta);
  return {...next,x:current.center.x-anchor.x*ratio,y:current.center.y-(anchor.y+groundLift*camera.zoom)*ratio};
}
// valueOf(entity) -> finite number|null selects the measured value per surface
// (default: observed maximum method complexity); zOf(value) -> plate height lets
// metric lenses normalise heights to the highest value in the current view.
function worldLayout(regions,valueOf,zOf){
  const metricValue=valueOf||(e=>Number.isFinite(e.max_complexity)?e.max_complexity:null),plate=value=>zOf?zOf(value):value===null?.08:.08+Math.log2(1+value)*.36;
  const rows=regions.slice().sort((a,b)=>a.name<b.name?-1:a.name>b.name?1:0),total=rows.reduce((n,r)=>n+r.entities.length,0),batch=Math.max(1,Math.ceil(total/750));
  const cells=[],areas=[];let x=0,y=0,rowHeight=0;
  const width=Math.max(8,Math.sqrt(Math.min(total,750))*1.4);
  for(const region of rows){
    const entities=region.entities.slice().sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0),n=Math.ceil(entities.length/batch),cols=Math.max(1,Math.ceil(Math.sqrt(n))),height=Math.ceil(n/cols);
    if(x&&x+cols>width){x=0;y+=rowHeight+2;rowHeight=0;}
    const area={name:region.name,x,y,w:cols,h:Math.max(1,height),region};areas.push(area);
    for(let i=0;i<n;i++){
      const members=entities.slice(i*batch,(i+1)*batch),known=members.map(metricValue).filter(Number.isFinite),value=known.length?Math.max(...known):null;
      cells.push({x:x+i%cols,y:y+Math.floor(i/cols),z:plate(value),value,area:region.name,entities:members});
    }
    x+=cols+2;rowHeight=Math.max(rowHeight,height);
  }
  return {areas,cells};
}
function worldProject(x,y,z=0,rotation=0){const c=Math.cos(rotation),s=Math.sin(rotation),rx=x*c-y*s,ry=x*s+y*c;return {x:(rx-ry)*.86,y:(rx+ry)*.44-z};}
function worldContains(point,polygon){let inside=false;for(let i=0,j=polygon.length-1;i<polygon.length;j=i++){const a=polygon[i],b=polygon[j];if((a.y>point.y)!==(b.y>point.y)&&point.x<(b.x-a.x)*(point.y-a.y)/(b.y-a.y)+a.x)inside=!inside;}return inside;}
