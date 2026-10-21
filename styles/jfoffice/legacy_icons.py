"""Preserved jfOffice geometry for non-Office SGF1 fallback symbols.

The active generator is mkicons.py. Coloured Office roles live in icon_art.py.
"""
import math

def line(x, y, a, b, width=75):
    dx, dy = a-x, b-y
    scale = width / (2*math.hypot(dx, dy))
    ox, oy = -dy*scale, dx*scale
    return [(round(x+ox),round(y+oy)),(round(a+ox),round(b+oy)),
            (round(a-ox),round(b-oy)),(round(x-ox),round(y-oy))]

def lines(points):
    return [line(*a,*b) for a,b in zip(points,points[1:])]

def box(x,y,w,h):
    return lines([(x,y),(x+w,y),(x+w,y+h),(x,y+h),(x,y)])

def circle(x, y, radius, steps=24):
    points = [(round(x+radius*math.cos(i*2*math.pi/steps)),
               round(y+radius*math.sin(i*2*math.pi/steps))) for i in range(steps)]
    return lines(points + [points[0]])

shapes = {}
def add(names, contours):
    for name in names.split(): shapes['IC_'+name] = contours

add('CHROME_CLOSE CANCEL ERROR',lines([(240,240),(760,760)])+lines([(240,760),(760,240)]))
add('ADD',lines([(180,500),(820,500)])+lines([(500,180),(500,820)]))
add('REMOVE CHROME_MIN',lines([(220,320),(780,320)]))
add('CHROME_MAX FULL_SCREEN',box(200,200,600,600))
add('CHROME_RESTORE BACK_TO_WINDOW COPY',box(180,180,460,460)+lines([(360,720),(820,720),(820,360)]))
add('CHEV_DOWN CHEV_DOWN_SMALL CARET_DOWN DOWN',lines([(230,660),(500,370),(770,660)]))
add('CHEV_UP CHEV_UP_SMALL CARET_UP UP',lines([(230,340),(500,630),(770,340)]))
add('CHEV_LEFT CHEV_LEFT_SMALL CARET_LEFT BACK PREVIOUS',lines([(650,230),(360,500),(650,770)]))
add('CHEV_RIGHT CHEV_RIGHT_SMALL CARET_RIGHT FORWARD NEXT',lines([(350,230),(640,500),(350,770)]))
add('CHECK_MARK',lines([(180,480),(410,250),(830,750)]))
add('MENU',sum((lines([(180,y),(820,y)]) for y in (250,500,750)),[]))
add('DOCUMENT PAGE TEXT_BOX',box(240,120,520,760)+lines([(340,620),(660,620)])+lines([(340,450),(660,450)]))
add('FOLDER OPEN_FILE',lines([(130,240),(850,240),(850,660),(530,660),(430,800),(130,800),(130,240)]))
add('SAVE SAVE_AS',box(180,150,640,700)+box(300,570,400,280)+box(310,150,380,270))
add('UNDO',lines([(200,580),(630,580),(790,420),(790,200)])+lines([(410,790),(200,580),(410,370)]))
add('REDO',lines([(800,580),(370,580),(210,420),(210,200)])+lines([(590,790),(800,580),(590,370)]))
add('DELETE',box(290,180,420,530)+lines([(180,760),(820,760)])+lines([(380,880),(620,880)]))
add('CHECKBOX',box(180,180,640,640))
add('CHECKBOX_FILL CHECKBOX_COMPOSITE',shapes['IC_CHECKBOX']+shapes['IC_CHECK_MARK'])
add('CHECKBOX_INDET',shapes['IC_CHECKBOX']+lines([(320,500),(680,500)]))
add('SIZE_GRIP',lines([(420,180),(820,580)])+lines([(660,180),(820,340)]))
add('PLAY', [[(280,180),(280,820),(820,500)]])
add('PAUSE', [line(350,200,350,800,140),line(650,200,650,800,140)])
add('STOP', [[(250,250),(250,750),(750,750),(750,250)]])
add('MORE', [line(x,470,x,530,80) for x in (220,500,780)])
add('BOLD', lines([(290,150),(290,850),(590,850),(740,740),(740,610),(620,510),
                  (290,510),(620,510),(760,400),(760,260),(610,150),(290,150)]))
add('ITALIC', lines([(360,150),(640,850)])+lines([(420,850),(790,850)])+
    lines([(190,150),(560,150)]))
add('UNDERLINE', lines([(250,850),(250,410),(310,300),(500,250),(690,300),(750,410),(750,850)])+
    lines([(180,100),(820,100)]))
add('STRIKETHROUGH', lines([(740,780),(610,850),(370,850),(250,750),(250,610),
                           (360,510),(650,430),(750,320),(750,230),(630,150),(370,150),(240,220)])+
    lines([(120,500),(880,500)]))
add('ALIGN_LEFT', sum((lines([(170,y),(170+w,y)]) for y,w in [(800,660),(600,450),(400,660),(200,450)]),[]))
add('ALIGN_CENTER', sum((lines([(500-w//2,y),(500+w//2,y)]) for y,w in [(800,660),(600,450),(400,660),(200,450)]),[]))
add('ALIGN_RIGHT', sum((lines([(830-w,y),(830,y)]) for y,w in [(800,660),(600,450),(400,660),(200,450)]),[]))
add('BULLETED_LIST', sum((lines([(350,y),(830,y)])+[line(180,y-25,180,y+25,90)] for y in (750,500,250)),[]))
add('SEARCH ZOOM',circle(440,570,240)+lines([(620,390),(850,160)]))
add('ZOOM_IN',shapes['IC_SEARCH']+lines([(290,570),(590,570)])+lines([(440,420),(440,720)]))
add('ZOOM_OUT',shapes['IC_SEARCH']+lines([(290,570),(590,570)]))
add('EDIT PENCIL',lines([(200,190),(250,390),(720,860),(860,720),(390,250),(200,190)])+
    lines([(620,760),(760,620)]))
add('LINK',lines([(450,650),(600,800),(770,800),(860,710),(860,540),(700,380)])+
    lines([(550,350),(400,200),(230,200),(140,290),(140,460),(300,620)])+
    lines([(340,390),(660,710)]))
add('SLIDERS SETTINGS',sum((lines([(160,y),(840,y)])+box(x-60,y-100,120,200)
    for x,y in [(340,760),(670,500),(430,240)]),[]))
add('VIEW VIEW_ALL',box(140,220,720,560)+lines([(340,220),(340,780)])+
    lines([(450,620),(730,620)])+lines([(450,440),(730,440)]))
add('FIT_PAGE',box(320,150,360,700)+lines([(100,660),(100,870),(240,870)])+
    lines([(760,870),(900,870),(900,660)])+lines([(100,340),(100,130),(240,130)])+
    lines([(760,130),(900,130),(900,340)]))
add('HOME',lines([(120,520),(500,870),(880,520)])+lines([(240,620),(240,160),(760,160),(760,620)])+
    lines([(420,160),(420,420),(580,420),(580,160)]))
add('HISTORY RECENT',circle(500,500,330)+lines([(500,730),(500,500),(700,390)]))
add('INFO HELP',circle(500,500,330)+lines([(500,270),(500,540)])+[line(500,680,500,700,85)])
add('WARNING',lines([(500,880),(100,150),(900,150),(500,880)])+
    lines([(500,650),(500,420)])+[line(500,270,500,290,90)])
add('DOWNLOAD',lines([(500,880),(500,330)])+lines([(290,540),(500,330),(710,540)])+
    lines([(170,330),(170,140),(830,140),(830,330)]))
add('UPLOAD',lines([(500,330),(500,880)])+lines([(290,670),(500,880),(710,670)])+
    lines([(170,330),(170,140),(830,140),(830,330)]))
add('PERSON',circle(500,710,150)+lines([(180,160),(230,350),(350,440),(650,440),(770,350),(820,160)]))
add('PIN',lines([(300,840),(700,840),(630,540),(790,370),(210,370),(370,540),(300,840)])+
    lines([(500,370),(500,100)]))
add('CUT',circle(260,260,135)+circle(740,260,135)+
    lines([(350,370),(760,860)])+lines([(650,370),(240,860)]))
add('PASTE',box(240,120,590,680)+box(350,710,330,150)+
    lines([(360,540),(700,540)])+lines([(360,360),(640,360)]))
add('FONT FONT_SIZE',lines([(170,150),(450,860),(730,150)])+
    lines([(280,430),(620,430)]))
add('FONT_COLOR',shapes['IC_FONT']+[line(130,70,800,70,110)])
add('FONT_INCREASE',lines([(130,150),(350,790),(580,150)])+
    lines([(230,420),(490,420)])+lines([(790,450),(790,850)])+
    lines([(650,700),(790,850),(930,700)]))
add('FONT_DECREASE',lines([(130,150),(350,790),(580,150)])+
    lines([(230,420),(490,420)])+lines([(790,850),(790,450)])+
    lines([(650,600),(790,450),(930,600)]))
add('BRUSH',lines([(310,450),(680,870),(810,750),(470,330),(310,450)])+
    lines([(320,410),(210,340),(190,170),(80,100),(300,110),(430,250),(430,330)]))
add('ERASER CLEAR',lines([(160,430),(570,860),(860,580),(430,150),(290,150),(160,280),(160,430)])+
    lines([(310,580),(600,300)])+lines([(420,150),(870,150)]))
add('INKING_TOOL CALLIGRAPHY_PEN',lines([(240,180),(370,550),(720,870),(870,720),(550,370),(240,180)])+
    lines([(240,180),(590,520)])+circle(610,550,55))
add('HIGHLIGHT CRAYON',lines([(220,270),(270,500),(700,880),(880,700),(500,270),(220,270)])+
    lines([(270,500),(500,270)])+[line(120,120,830,120,110)])
add('DROP',lines([(500,880),(240,510),(190,330),(270,170),(500,100),(730,170),(810,330),(760,510),(500,880)]))
add('EYEDROPPER',lines([(170,130),(250,360),(700,810),(810,700),(360,250),(170,130)])+
    lines([(570,850),(850,570)]))
add('PHOTO IMAGE PICTURES',box(140,180,730,650)+
    lines([(160,230),(380,490),(550,330),(700,570),(850,390)])+circle(370,680,70))
add('GRID',box(150,160,700,680)+lines([(150,620),(850,620)])+
    lines([(150,390),(850,390)])+lines([(380,160),(380,840)])+lines([(610,160),(610,840)]))
add('PRINT',box(250,590,500,280)+box(120,280,760,370)+box(250,120,500,310)+
    lines([(350,300),(650,300)])+lines([(350,220),(650,220)]))
add('LOCK',box(230,150,540,440)+lines([(340,590),(340,780),(420,870),(580,870),(660,780),(660,590)])+
    lines([(500,310),(500,450)]))
add('UNLOCK',box(230,150,540,440)+lines([(660,590),(660,780),(580,870),(420,870),(340,780)])+
    lines([(500,310),(500,450)]))
add('SELECT_ALL SELECT_RECT',sum((lines(p) for p in [
    [(160,590),(160,840),(400,840)],[(600,840),(840,840),(840,590)],
    [(840,400),(840,160),(600,160)],[(400,160),(160,160),(160,400)]]),[]))
add('SHAPES',box(150,420,450,450)+circle(640,350,250))
add('POINTER',lines([(200,850),(280,180),(470,390),(680,100),(810,190),(610,470),(860,500),(200,850)]))
add('ACCEPT',shapes['IC_CHECK_MARK'])
add('OPEN_IN_NEW_WINDOW',lines([(220,740),(220,220),(740,220)])+
    lines([(460,540),(830,910),(560,910)])+lines([(830,910),(830,640)]))
