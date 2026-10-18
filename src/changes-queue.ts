// Cold metadata/patch requests share one native slot. Cache hits bypass it.
export class ChangesQueue {
  private tail:Promise<void>=Promise.resolve();
  async acquire():Promise<()=>void>{
    const previous=this.tail;
    let release!:()=>void;
    this.tail=new Promise<void>(resolve=>{release=resolve;});
    await previous;
    return release;
  }
}
