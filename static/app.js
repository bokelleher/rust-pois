(function(){
  function getToken(){ return localStorage.getItem("pois_token") || ""; }
  function setToken(t){ 
    if(t) localStorage.setItem("pois_token", t); 
    else localStorage.removeItem("pois_token"); 
    updateTokenDisplay(); 
  }
  function updateTokenDisplay(){ 
    var el = document.getElementById("tokenDisplay"); 
    if(!el) return; 
    var t = getToken(); 
    el.textContent = t ? ("token: " + t.substring(0, 8) + "...") : "token: unset"; 
  }
  window.POIS_TOKEN = { get: getToken, set: setToken, refresh: updateTokenDisplay };
  document.addEventListener("DOMContentLoaded", updateTokenDisplay);
  window.addEventListener("storage", function(e){ 
    if(e.key === "pois_token") updateTokenDisplay(); 
  });
})();