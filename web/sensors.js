// The motion sensors and the location for the worker (lib/plat/sensors.rs, web_sensors.rs):
// - REQUEST_SENSORS (u32 SENSOR_* union): devicemotion (acceleration with gravity, gravity as the
//   difference to the acceleration without it, rotationRate) and deviceorientation (a quaternion;
//   absolute with deviceorientationabsolute or Safari's webkitCompassHeading, else relative to
//   where it started); the magnetic field does not exist here.  Safari's requestPermission is
//   asked from the click that started it; when the browser wants a gesture of its own, the next
//   tap asks.  What gave nothing after NO_DATA_MS (a computer without sensors; Chromium's first
//   event with null values) is Unavailable until an event says otherwise;
// - REQUEST_LOCATION (u32 0 stop, 1 start, 2 start again after the background):
//   navigator.geolocation.watchPosition, the permission's change event (taken back: Denied, given
//   again: the watch starts anew); a page without a secure context has no location.
// Readings go to the worker at most every READ_MS per sensor as {type:"sensor"}, states as
// {type:"sensor-state"}, the location as {type:"location"}.  document.body.dataset.sensors and
// .location say what runs (tests).
const REQUEST_SENSORS=26, REQUEST_LOCATION=27;
const ACCEL=1, GRAVITY=2, GYRO=4, MAGNET=8, ROTATION=16, MOTION=ACCEL|GRAVITY|GYRO;
const WAITING=1, DENIED=4, UNAVAILABLE=6;
const SEARCHING=1, FIX=2, LOC_DENIED=4, LOC_DISABLED=5, LOC_UNAVAILABLE=6;
const READ_MS=33, NO_DATA_MS=1500;
const G2R=Math.PI/180;
// iOS reports the acceleration with the opposite sign of the specification (and of Android)
const flip=/iPhone|iPad|iPod/.test(navigator.userAgent)?-1:1;

// alpha, beta, gamma (degrees: z, x', y'') -> the quaternion device -> east, north, up (W3C's
// worked example)
function quaternion(a,b,g) {
  const c=d=>Math.cos(d*G2R/2), s=d=>Math.sin(d*G2R/2);
  const [cz,cx,cy,sz,sx,sy]=[c(a),c(b),c(g),s(a),s(b),s(g)];
  return [sx*cy*cz-cx*sy*sz, cx*sy*cz+sx*cy*sz, cx*cy*sz+sx*sy*cz, cx*cy*cz-sx*sy*sz];
}

export class SensorHost {
  constructor(send,canvas) {
    this.send=send; this.canvas=canvas; this.mask=0; this.got=0; this.last={}; this.timer=0;
    this.motion=e=>this.onMotion(e); this.orient=e=>this.onOrient(e,false); this.orientAbs=e=>this.onOrient(e,true);
    this.listening=false; this.absolute=false; this.watch=null; this.wanted=false; this.perm=null;
    this.permChange=()=>this.permissionChanged();
    document.body.dataset.sensors="0";
    document.body.dataset.location="off";
  }
  reading(bit,x,y,z,w,flags) {
    const now=performance.now();
    if(now-(this.last[bit]||0)<READ_MS) return;
    this.last[bit]=now;
    this.got|=bit;
    this.send({type:"sensor",bit,x,y,z,w,flags});
  }
  onMotion(e) {
    const a=e.accelerationIncludingGravity, l=e.acceleration, r=e.rotationRate;
    const num=(...v)=>v.every(x=>typeof x==="number" && isFinite(x));
    if(a && num(a.x,a.y,a.z)) {
      if(this.mask&ACCEL) this.reading(ACCEL,flip*a.x,flip*a.y,flip*a.z,0,3);
      if((this.mask&GRAVITY) && l && num(l.x,l.y,l.z)) this.reading(GRAVITY,flip*(a.x-l.x),flip*(a.y-l.y),flip*(a.z-l.z),0,3);
    }
    if((this.mask&GYRO) && r && num(r.alpha,r.beta,r.gamma)) this.reading(GYRO,r.beta*G2R,r.gamma*G2R,r.alpha*G2R,0,3);
  }
  onOrient(e,absolute) {
    if(!(this.mask&ROTATION) || typeof e.beta!=="number" || typeof e.gamma!=="number") return;
    let alpha=e.alpha;
    if(typeof e.webkitCompassHeading==="number") {alpha=360-e.webkitCompassHeading;absolute=true;}
    if(typeof alpha!=="number") return;
    if(absolute||e.absolute) this.absolute=true;
    // the absolute events win once they came
    else if(this.absolute) return;
    const q=quaternion(alpha,e.beta,e.gamma);
    this.reading(ROTATION,q[0],q[1],q[2],q[3],3|(absolute||e.absolute?4:0));
  }
  listen(on) {
    if(on===this.listening) return;
    this.listening=on;
    const op=on?"addEventListener":"removeEventListener";
    window[op]("devicemotion",this.motion);
    window[op]("deviceorientation",this.orient);
    window[op]("deviceorientationabsolute",this.orientAbs);
    if(!on) this.absolute=false;
  }
  // Safari asks here; from a worker's message the gesture may have ended: then the next tap asks
  async ask() {
    const want=[window.DeviceMotionEvent,window.DeviceOrientationEvent].filter(c=>typeof c?.requestPermission==="function");
    if(!want.length) return true;
    try {
      for(const c of want) if(await c.requestPermission()!=="granted") return false;
      return true;
    } catch(error) {
      if(error?.name!=="NotAllowedError") return false;
      this.canvas.addEventListener("pointerup",async()=>{
        if(!this.mask) return;
        if(await this.ask()) this.listen(true);
        else this.send({type:"sensor-state",mask:this.mask,state:DENIED});
      },{once:true});
      return null;
    }
  }
  async sensors(mask) {
    const started=mask&~this.mask;
    this.mask=mask;
    document.body.dataset.sensors=String(mask);
    clearTimeout(this.timer);
    if(!mask) {this.listen(false);this.got=0;return;}
    this.got&=mask;
    let missing=mask&MAGNET;
    if(!window.DeviceMotionEvent) missing|=mask&MOTION;
    if(!window.DeviceOrientationEvent) missing|=mask&ROTATION;
    if(missing) this.send({type:"sensor-state",mask:missing,state:UNAVAILABLE});
    if(mask&~missing && started) {
      const answer=await this.ask();
      if(answer===false) {this.send({type:"sensor-state",mask:mask&~missing,state:DENIED});return;}
      if(answer===true && this.mask) this.listen(true);
    }
    this.timer=setTimeout(()=>{
      const silent=this.mask&~this.got&~missing;
      if(silent) this.send({type:"sensor-state",mask:silent,state:UNAVAILABLE});
    },NO_DATA_MS);
  }
  // ---------------------------------------------------------------- location
  fix(p) {
    const c=p.coords, n=x=>x===null||x===undefined?NaN:x;
    document.body.dataset.location="fix";
    this.send({type:"location",state:FIX,lat:c.latitude,lon:c.longitude,accuracy:n(c.accuracy),altitude:n(c.altitude),
      speed:n(c.speed),bearing:n(c.heading),age:Math.max(0,Date.now()-p.timestamp),source:0});
  }
  state(state,name) {
    document.body.dataset.location=name;
    this.send({type:"location",state});
  }
  failed(e) {
    if(e.code===1) {this.stopWatch();this.state(LOC_DENIED,"denied");}
    else if(e.code===2) this.state(LOC_DISABLED,"disabled");
    // a timeout: the watch goes on
  }
  startWatch() {
    this.stopWatch();
    this.watch=navigator.geolocation.watchPosition(p=>this.fix(p),e=>this.failed(e),{enableHighAccuracy:true,maximumAge:1000});
  }
  stopWatch() {
    if(this.watch!==null) navigator.geolocation.clearWatch(this.watch);
    this.watch=null;
  }
  permissionChanged() {
    if(!this.wanted || !this.perm) return;
    // ("prompt": the user reset it; asked again from the next user action)
    if(this.perm.state!=="granted") {this.stopWatch();this.state(LOC_DENIED,"denied");}
    else if(this.perm.state==="granted" && this.watch===null) {this.state(SEARCHING,"searching");this.startWatch();}
  }
  async location(op) {
    this.wanted=op!==0;
    if(!op) {
      this.stopWatch();
      this.perm?.removeEventListener("change",this.permChange);
      this.perm=null;
      document.body.dataset.location="off";
      return;
    }
    if(!navigator.geolocation || !window.isSecureContext) {this.state(LOC_UNAVAILABLE,"unavailable");return;}
    this.state(SEARCHING,"searching");
    if(op===1) this.startWatch(); // from the user's gesture: the browser may ask
    let state="";
    try {
      const perm=await navigator.permissions?.query({name:"geolocation"});
      if(perm) {
        if(!this.wanted) return;
        this.perm?.removeEventListener("change",this.permChange);
        this.perm=perm;perm.addEventListener("change",this.permChange);state=perm.state;
      }
    } catch(error) {/* no Permissions API for it: the watch's error says it */}
    if(op!==2 || !this.wanted) return;
    // back from the background: asks nobody
    if(state==="prompt") this.state(LOC_DENIED,"denied");
    else this.startWatch();
  }
  // the worker's requests of web_sensors.rs -> true when it was one of them
  service(m) {
    const arg=()=>new DataView(m.data).getUint32(0,true);
    if(m.kind===REQUEST_SENSORS) this.sensors(arg()).catch(error=>console.warn("Sensoren:",error));
    else if(m.kind===REQUEST_LOCATION) this.location(arg()).catch(error=>console.warn("Standort:",error));
    else return false;
    return true;
  }
}
