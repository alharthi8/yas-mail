// Saved order contains account IDs only; provider account objects remain untouched.
export function restoreAccountOrder(accounts,savedIds){
  const ordered=[...accounts];
  if(!Array.isArray(savedIds))return ordered;
  const existing=new Set(accounts.map(account=>account.id));
  const rank=new Map();
  for(const id of savedIds){
    if(existing.has(id)&&!rank.has(id))rank.set(id,rank.size);
  }
  // Stable sorting keeps new/unlisted accounts in the order returned by the provider.
  return ordered.sort((left,right)=>(rank.get(left.id)??Infinity)-(rank.get(right.id)??Infinity));
}

export function moveAccount(accounts,id,targetIndex){
  const reordered=[...accounts];
  const currentIndex=reordered.findIndex(account=>account.id===id);
  if(currentIndex<0||typeof targetIndex!=='number'||Number.isNaN(targetIndex))return reordered;
  const destination=Math.max(0,Math.min(reordered.length-1,Math.trunc(targetIndex)));
  if(destination===currentIndex)return reordered;
  const [account]=reordered.splice(currentIndex,1);
  reordered.splice(destination,0,account);
  return reordered;
}
